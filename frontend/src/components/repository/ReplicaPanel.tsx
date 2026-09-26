import { type FormEvent, useState } from "react";
import { AlertCircle, Download, Save, Upload } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import type { ReplicaPolicyRequest } from "@/api/generated/ReplicaPolicyRequest";
import type { ReplicaPolicyResponse } from "@/api/generated/ReplicaPolicyResponse";
import {
  usePullReplica,
  usePushReplica,
  useReplicaPolicy,
  useSaveReplicaPolicy,
} from "@/api/queries";
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

const EMPTY: ReplicaPolicyResponse = {
  configured: false,
  remote_url: null,
  destination_id: null,
  direction: "push",
  has_token: false,
  interval_minutes: null,
  last_run: null,
};

export function ReplicaPanel({
  repositoryId,
  canWrite,
}: {
  repositoryId: string;
  canWrite: boolean;
}) {
  const { t } = useTranslation();
  const { data, isPending, isError, error } = useReplicaPolicy(repositoryId, true);

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("replica.title")}</CardTitle>
        <CardDescription>{t("replica.description")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {isPending ? <Skeleton className="h-24 w-full rounded-md" /> : null}
        {isError && error.message ? (
          <Alert variant="destructive">
            <AlertCircle />
            <AlertTitle>{t("replica.loadFailed")}</AlertTitle>
            <AlertDescription>{error.message}</AlertDescription>
          </Alert>
        ) : null}
        {isPending ? null : (
          <ReplicaForm
            repositoryId={repositoryId}
            canWrite={canWrite}
            policy={data ?? EMPTY}
          />
        )}
      </CardContent>
    </Card>
  );
}

function ReplicaForm({
  repositoryId,
  canWrite,
  policy,
}: {
  repositoryId: string;
  canWrite: boolean;
  policy: ReplicaPolicyResponse;
}) {
  const { t } = useTranslation();
  const savePolicy = useSaveReplicaPolicy(repositoryId);
  const pushReplica = usePushReplica(repositoryId);
  const pullReplica = usePullReplica(repositoryId);
  const [remoteUrl, setRemoteUrl] = useState(policy.remote_url ?? "");
  const [destinationId, setDestinationId] = useState(policy.destination_id ?? "");
  const [direction, setDirection] = useState(policy.direction === "pull" ? "pull" : "push");
  const [intervalMinutes, setIntervalMinutes] = useState(
    policy.interval_minutes == null ? "" : String(policy.interval_minutes),
  );
  const [token, setToken] = useState("");
  const pulling = direction === "pull";
  const running = pushReplica.isPending || pullReplica.isPending;

  async function onSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmedInterval = intervalMinutes.trim();
    let interval: number | undefined;
    if (trimmedInterval.length > 0) {
      const parsed = Number.parseInt(trimmedInterval, 10);
      if (!Number.isFinite(parsed) || parsed < 0 || parsed > 10080) {
        toast.error(t("replica.intervalHint"));
        return;
      }
      interval = parsed;
    } else {
      interval = 0;
    }
    const payload: ReplicaPolicyRequest = {
      remote_url: remoteUrl.trim() || undefined,
      destination_id: destinationId.trim() || undefined,
      token: token.trim() || undefined,
      direction,
      interval_minutes: interval,
    };
    try {
      await savePolicy.mutateAsync(payload);
      setToken("");
      toast.success(
        payload.remote_url && payload.destination_id
          ? t("replica.saved")
          : t("replica.cleared"),
      );
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("replica.saveFailed"));
    }
  }

  async function onReplicate() {
    try {
      const outcome = pulling
        ? await pullReplica.mutateAsync()
        : await pushReplica.mutateAsync();
      toast.success(
        t(pulling ? "replica.pulled" : "replica.pushed", {
          packages: outcome.packages_imported,
          artifacts: outcome.artifacts_imported,
          skipped: outcome.skipped,
        }),
      );
    } catch (err) {
      toast.error(
        err instanceof ApiError
          ? err.message
          : t(pulling ? "replica.pullFailed" : "replica.pushFailed"),
      );
    }
  }

  return (
    <form onSubmit={(event) => void onSave(event)} className="space-y-5">
      <div className="space-y-2">
        <Label htmlFor="replica-direction">{t("replica.direction")}</Label>
        <Select
          value={direction}
          onValueChange={(value) => setDirection(value === "pull" ? "pull" : "push")}
          disabled={!canWrite}
        >
          <SelectTrigger id="replica-direction" className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="push">{t("replica.directionPush")}</SelectItem>
            <SelectItem value="pull">{t("replica.directionPull")}</SelectItem>
          </SelectContent>
        </Select>
        <p className="text-sm text-muted-foreground">{t("replica.directionHint")}</p>
      </div>
      <div className="space-y-2">
        <Label htmlFor="replica-url">{t("replica.remoteUrl")}</Label>
        <Input
          id="replica-url"
          value={remoteUrl}
          onChange={(event) => setRemoteUrl(event.target.value)}
          placeholder="http://127.0.0.1:3000"
          disabled={!canWrite}
          autoComplete="off"
        />
        <p className="text-sm text-muted-foreground">
          {t(pulling ? "replica.remoteUrlHintPull" : "replica.remoteUrlHintPush")}
        </p>
      </div>
      <div className="space-y-2">
        <Label htmlFor="replica-destination">
          {t(pulling ? "replica.destinationPull" : "replica.destination")}
        </Label>
        <Input
          id="replica-destination"
          value={destinationId}
          onChange={(event) => setDestinationId(event.target.value)}
          placeholder="8-4-4-4-12"
          disabled={!canWrite}
          className="font-mono"
          autoComplete="off"
        />
        <p className="text-sm text-muted-foreground">
          {t(pulling ? "replica.destinationHintPull" : "replica.destinationHint")}
        </p>
      </div>
      <div className="space-y-2">
        <Label htmlFor="replica-token">{t("replica.token")}</Label>
        <Input
          id="replica-token"
          type="password"
          value={token}
          onChange={(event) => setToken(event.target.value)}
          placeholder={
            policy.has_token
              ? t("replica.tokenKept")
              : t(pulling ? "replica.tokenPlaceholderPull" : "replica.tokenPlaceholder")
          }
          disabled={!canWrite}
          autoComplete="off"
        />
        <p className="text-sm text-muted-foreground">
          {t(pulling ? "replica.tokenHintPull" : "replica.tokenHint")}
        </p>
      </div>
      <div className="space-y-2">
        <Label htmlFor="replica-interval">{t("replica.interval")}</Label>
        <Input
          id="replica-interval"
          type="number"
          min={0}
          max={10080}
          value={intervalMinutes}
          onChange={(event) => setIntervalMinutes(event.target.value)}
          placeholder={t("replica.intervalOff")}
          disabled={!canWrite}
        />
        <p className="text-sm text-muted-foreground">{t("replica.intervalHint")}</p>
      </div>

      {policy.last_run ? (
        <div className="space-y-1 rounded-md border border-border bg-muted/40 px-3 py-2">
          <p className="text-sm font-medium text-foreground">{t("replica.lastRun")}</p>
          {policy.last_run.error ? (
            <p className="text-sm text-destructive">{policy.last_run.error}</p>
          ) : (
            <p className="text-sm text-muted-foreground">
              {t("replica.lastRunOk", {
                packages: policy.last_run.packages_imported,
                artifacts: policy.last_run.artifacts_imported,
                skipped: policy.last_run.skipped,
              })}
            </p>
          )}
          <p className="text-xs text-muted-foreground">{policy.last_run.occurred_at}</p>
        </div>
      ) : null}

      {canWrite ? (
        <div className="flex flex-wrap gap-2">
          <Button type="submit" variant="outline" disabled={savePolicy.isPending}>
            <Save />
            {savePolicy.isPending ? t("common.saving") : t("common.save")}
          </Button>
          <Button
            type="button"
            disabled={running || !policy.configured}
            onClick={() => void onReplicate()}
          >
            {pulling ? <Download /> : <Upload />}
            {running ? t("replica.pushing") : t("replica.pushNow")}
          </Button>
        </div>
      ) : (
        <p className="text-sm text-muted-foreground">{t("replica.readOnly")}</p>
      )}
    </form>
  );
}
