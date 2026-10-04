import { type FormEvent, useEffect, useState } from "react";
import { KeyRound, Loader2, Save, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import {
  useClearMirrorUpstreamAuth,
  useMirrorUpstreamAuth,
  useSetMirrorUpstreamAuth,
} from "@/api/queries";
import { ConfirmDeleteDialog } from "@/components/repository/ConfirmDeleteDialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export function MirrorUpstreamAuthPanel({
  repositoryId,
  canWrite,
}: {
  repositoryId: string;
  canWrite: boolean;
}) {
  const { t } = useTranslation();
  const status = useMirrorUpstreamAuth(repositoryId);
  const save = useSetMirrorUpstreamAuth(repositoryId);
  const clear = useClearMirrorUpstreamAuth(repositoryId);
  const [username, setUsername] = useState("");
  const [secret, setSecret] = useState("");

  useEffect(() => {
    const saved = status.data?.username ?? "";
    queueMicrotask(() => setUsername(saved));
  }, [status.data?.username]);

  const configured = status.data?.configured === true;
  const savedUsername = status.data?.username ?? "";
  const statusText = !status.data
    ? t("repositories.upstreamAuthLoading")
    : configured
      ? savedUsername.length > 0
        ? t("repositories.upstreamAuthAsUser", { username: savedUsername })
        : t("repositories.upstreamAuthAsBearer")
      : t("repositories.upstreamAuthNone");

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (secret.trim().length === 0) {
      toast.error(t("repositories.upstreamAuthSecretRequired"));
      return;
    }
    try {
      await save.mutateAsync({ username: username.trim(), secret });
      setSecret("");
      toast.success(t("repositories.upstreamAuthSaved"));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("repositories.upstreamAuthFailed"));
    }
  }

  async function onRemove() {
    try {
      await clear.mutateAsync();
      setSecret("");
      toast.success(t("repositories.upstreamAuthRemoved"));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("repositories.upstreamAuthFailed"));
      throw err;
    }
  }

  return (
    <form
      onSubmit={(event) => void onSubmit(event)}
      className="space-y-3 rounded-lg border border-border bg-card px-3 py-3"
    >
      <div className="flex items-center gap-2">
        <KeyRound className="size-4 text-muted-foreground" />
        <p className="text-sm font-medium">{t("repositories.upstreamAuth")}</p>
      </div>
      <p className="text-xs text-muted-foreground">{t("repositories.upstreamAuthHint")}</p>
      <p className="text-xs text-muted-foreground">{statusText}</p>
      <div className="flex flex-wrap items-end gap-3">
        <div className="min-w-[10rem] space-y-1">
          <Label htmlFor="mirror-upstream-username">{t("repositories.upstreamAuthUsername")}</Label>
          <Input
            id="mirror-upstream-username"
            autoComplete="off"
            placeholder={t("repositories.upstreamAuthUsernameHint")}
            value={username}
            onChange={(event) => setUsername(event.target.value)}
            disabled={!canWrite || save.isPending}
          />
        </div>
        <div className="min-w-[12rem] space-y-1">
          <Label htmlFor="mirror-upstream-secret">{t("repositories.upstreamAuthSecret")}</Label>
          <Input
            id="mirror-upstream-secret"
            type="password"
            autoComplete="new-password"
            value={secret}
            onChange={(event) => setSecret(event.target.value)}
            disabled={!canWrite || save.isPending}
          />
        </div>
        {canWrite ? (
          <Button type="submit" size="sm" disabled={save.isPending || secret.trim().length === 0}>
            {save.isPending ? <Loader2 className="animate-spin" /> : <Save />}
            {t("repositories.upstreamAuthSave")}
          </Button>
        ) : null}
        {canWrite && configured ? (
          <ConfirmDeleteDialog
            title={t("repositories.upstreamAuthRemoveTitle")}
            description={t("repositories.upstreamAuthRemoveHint")}
            confirmLabel={t("repositories.upstreamAuthRemove")}
            pending={clear.isPending}
            onConfirm={onRemove}
            trigger={
              <Button type="button" size="sm" variant="outline" disabled={clear.isPending}>
                {clear.isPending ? <Loader2 className="animate-spin" /> : <Trash2 />}
                {t("repositories.upstreamAuthRemove")}
              </Button>
            }
          />
        ) : null}
      </div>
    </form>
  );
}
