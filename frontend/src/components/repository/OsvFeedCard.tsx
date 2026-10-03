import { type FormEvent, useEffect, useState } from "react";
import { Loader2, RefreshCw, Save } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useOsvFeed, useOsvSync, useRunOsvSync, useSaveOsvSync } from "@/api/queries";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

function importedLabel(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleString();
}

export function OsvFeedCard() {
  const { t } = useTranslation();
  const feed = useOsvFeed(true);
  const sync = useOsvSync(true);
  const save = useSaveOsvSync();
  const run = useRunOsvSync();
  const [reference, setReference] = useState("");
  const [publicKey, setPublicKey] = useState("");
  const [syncing, setSyncing] = useState(false);

  useEffect(() => {
    const nextReference = sync.data?.reference ?? "";
    const nextKey = sync.data?.public_key_pem ?? "";
    // Deferred via queueMicrotask instead of calling setState synchronously
    // in the effect body (react-hooks/set-state-in-effect).
    queueMicrotask(() => {
      setReference(nextReference);
      setPublicKey(nextKey);
    });
  }, [sync.data?.public_key_pem, sync.data?.reference]);

  async function persist(): Promise<boolean> {
    try {
      await save.mutateAsync({
        reference: reference.trim(),
        public_key_pem: publicKey.trim(),
      });
      return true;
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("feed.saveFailed"));
      return false;
    }
  }

  async function onSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (await persist()) {
      toast.success(t("feed.saved"));
    }
  }

  async function onSyncNow() {
    setSyncing(true);
    try {
      if (!(await persist())) {
        return;
      }
      const result = await run.mutateAsync();
      if (result.last_outcome === "imported") {
        toast.success(t("feed.syncImported", { dataset: result.last_detail ?? "" }));
      } else if (result.last_outcome === "unchanged") {
        toast.success(t("feed.syncUnchanged"));
      } else if (result.last_outcome === "rejected") {
        toast.error(result.last_detail ?? t("feed.syncRejected"));
      }
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("feed.syncFailed"));
    } finally {
      setSyncing(false);
    }
  }

  const busy = save.isPending || run.isPending || syncing;
  const rejected = sync.data?.last_outcome === "rejected";

  return (
    <Card className="gap-3 py-4">
      <CardHeader className="px-4">
        <CardTitle className="text-sm">{t("feed.title")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-4 px-4 text-sm">
        {feed.isPending ? (
          <p className="text-muted-foreground">{t("feed.loading")}</p>
        ) : feed.isError ? (
          <p className="text-destructive">{t("feed.loadFailed")}</p>
        ) : feed.data ? (
          <dl className="grid gap-2 sm:grid-cols-3">
            <div>
              <dt className="text-xs text-muted-foreground">{t("feed.dataset")}</dt>
              <dd className="font-mono text-foreground">{feed.data.dataset}</dd>
            </div>
            <div>
              <dt className="text-xs text-muted-foreground">{t("feed.imported")}</dt>
              <dd className="text-foreground">{importedLabel(feed.data.imported_at)}</dd>
            </div>
            <div>
              <dt className="text-xs text-muted-foreground">{t("feed.mode")}</dt>
              <dd className="font-mono text-foreground">{feed.data.mode}</dd>
            </div>
          </dl>
        ) : (
          <p className="text-muted-foreground">{t("feed.empty")}</p>
        )}

        {sync.isPending ? null : sync.isError ? (
          <p className="text-destructive">{t("feed.loadFailed")}</p>
        ) : (
          <form onSubmit={(event) => void onSave(event)} className="space-y-3">
            <div className="space-y-1">
              <Label htmlFor="osv-sync-reference">{t("feed.reference")}</Label>
              <Input
                id="osv-sync-reference"
                value={reference}
                placeholder={t("feed.placeholder")}
                onChange={(event) => setReference(event.target.value)}
                disabled={busy}
                spellCheck={false}
              />
              <p className="text-xs text-muted-foreground">{t("feed.hint")}</p>
            </div>
            <div className="space-y-1">
              <Label htmlFor="osv-sync-public-key">{t("feed.publicKey")}</Label>
              <textarea
                id="osv-sync-public-key"
                value={publicKey}
                rows={5}
                spellCheck={false}
                disabled={busy}
                onChange={(event) => setPublicKey(event.target.value)}
                className="min-h-28 w-full rounded-md border border-input bg-transparent px-3 py-2 font-mono text-xs shadow-xs outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50 dark:bg-input/30"
              />
            </div>
            {sync.data?.last_outcome ? (
              <p className="text-xs">
                <span className="text-muted-foreground">{t("feed.lastResult")}: </span>
                <span className="font-mono text-foreground">{sync.data.last_outcome}</span>
                {sync.data.last_detail ? (
                  <span className={rejected ? "text-destructive" : "text-foreground"}>
                    {" "}
                    {sync.data.last_detail}
                  </span>
                ) : null}
              </p>
            ) : null}
            <div className="flex flex-wrap gap-2">
              <Button type="submit" size="sm" disabled={busy}>
                {save.isPending && !syncing ? <Loader2 className="animate-spin" /> : <Save />}
                {save.isPending && !syncing ? t("feed.saving") : t("feed.save")}
              </Button>
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={busy}
                onClick={() => void onSyncNow()}
              >
                {syncing ? <Loader2 className="animate-spin" /> : <RefreshCw />}
                {syncing ? t("feed.syncing") : t("feed.syncNow")}
              </Button>
            </div>
          </form>
        )}
      </CardContent>
    </Card>
  );
}
