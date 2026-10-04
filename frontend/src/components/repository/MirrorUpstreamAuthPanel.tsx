import { type FormEvent, useEffect, useState } from "react";
import { KeyRound, Link2, Loader2, Plug, Save, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import {
  useClearMirrorUpstreamAuth,
  useMirrorUpstreamAuth,
  useProbeMirrorUpstreamAuth,
  useSetMirrorUpstream,
  useSetMirrorUpstreamAuth,
} from "@/api/queries";
import { ConfirmDeleteDialog } from "@/components/repository/ConfirmDeleteDialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export function MirrorUpstreamAuthPanel({
  repositoryId,
  upstream,
  canWrite,
}: {
  repositoryId: string;
  upstream: string;
  canWrite: boolean;
}) {
  const { t } = useTranslation();
  const status = useMirrorUpstreamAuth(repositoryId);
  const saveUrl = useSetMirrorUpstream(repositoryId);
  const save = useSetMirrorUpstreamAuth(repositoryId);
  const clear = useClearMirrorUpstreamAuth(repositoryId);
  const probe = useProbeMirrorUpstreamAuth(repositoryId);
  const [upstreamUrl, setUpstreamUrl] = useState(upstream);
  const [username, setUsername] = useState("");
  const [secret, setSecret] = useState("");

  useEffect(() => {
    queueMicrotask(() => setUpstreamUrl(upstream));
  }, [upstream]);

  useEffect(() => {
    const saved = status.data?.username ?? "";
    queueMicrotask(() => setUsername(saved));
  }, [status.data?.username]);

  const configured = status.data?.configured === true;
  const savedUsername = status.data?.username ?? "";
  const urlDirty = upstreamUrl.trim() !== upstream;
  const credentialsDirty = secret.length > 0 || username.trim() !== savedUsername;
  const statusText = !status.data
    ? t("repositories.upstreamAuthLoading")
    : configured
      ? savedUsername.length > 0
        ? t("repositories.upstreamAuthAsUser", { username: savedUsername })
        : t("repositories.upstreamAuthAsBearer")
      : t("repositories.upstreamAuthNone");
  const testBlocked = !configured
    ? t("repositories.upstreamAuthTestNeedCredential")
    : urlDirty
      ? t("repositories.upstreamAuthTestNeedSave")
      : credentialsDirty
        ? t("repositories.upstreamAuthTestUnsaved")
        : null;
  const canTest = canWrite && testBlocked == null && !probe.isPending;

  async function onSaveUrl(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const next = upstreamUrl.trim();
    if (next.length === 0) {
      toast.error(t("repositories.upstreamInvalid"));
      return;
    }
    try {
      await saveUrl.mutateAsync({ upstream: next });
      toast.success(t("repositories.upstreamUrlSaved"));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("repositories.upstreamUrlFailed"));
    }
  }

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

  async function onTest() {
    try {
      const result = await probe.mutateAsync();
      const message = result.authenticated
        ? t("repositories.upstreamAuthTestResult", { status: result.status })
        : t("repositories.upstreamAuthTestAnonymous", { status: result.status });
      if (result.ok) {
        toast.success(message);
      } else {
        toast.error(message);
      }
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("repositories.upstreamAuthTestFailed"));
    }
  }

  return (
    <div className="space-y-4 rounded-lg border border-border bg-card px-3 py-3">
      <form onSubmit={(event) => void onSaveUrl(event)} className="space-y-2">
        <div className="flex items-center gap-2">
          <Link2 className="size-4 text-muted-foreground" />
          <p className="text-sm font-medium">{t("repositories.upstreamUrl")}</p>
        </div>
        <p className="text-xs text-muted-foreground">{t("repositories.upstreamUrlHint")}</p>
        <div className="flex flex-wrap items-end gap-3">
          <div className="min-w-[16rem] flex-1 space-y-1">
            <Label htmlFor="mirror-upstream-url" className="sr-only">
              {t("repositories.upstreamUrl")}
            </Label>
            <Input
              id="mirror-upstream-url"
              type="url"
              autoComplete="off"
              value={upstreamUrl}
              onChange={(event) => setUpstreamUrl(event.target.value)}
              disabled={!canWrite || saveUrl.isPending}
            />
          </div>
          {canWrite ? (
            <Button type="submit" size="sm" disabled={saveUrl.isPending || !urlDirty}>
              {saveUrl.isPending ? <Loader2 className="animate-spin" /> : <Save />}
              {t("repositories.upstreamUrlSave")}
            </Button>
          ) : null}
        </div>
      </form>

      <form onSubmit={(event) => void onSubmit(event)} className="space-y-3">
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
          {canWrite ? (
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={!canTest}
              title={testBlocked ?? t("repositories.upstreamAuthTestHint")}
              onClick={() => void onTest()}
            >
              {probe.isPending ? <Loader2 className="animate-spin" /> : <Plug />}
              {t("repositories.upstreamAuthTest")}
            </Button>
          ) : null}
        </div>
        <p className="text-xs text-muted-foreground">
          {testBlocked ?? t("repositories.upstreamAuthTestHint")}
        </p>
      </form>
    </div>
  );
}
