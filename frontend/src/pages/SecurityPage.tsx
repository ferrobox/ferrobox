import { type FormEvent, useState } from "react";
import { AlertCircle, Copy, KeyRound, RefreshCw, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useApiTokens, useCreateApiToken, useRepositories, useRevokeApiToken } from "@/api/queries";
import { PageHeader } from "@/components/layout/PageHeader";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
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

export function SecurityPage() {
  const { t } = useTranslation();
  const { data, isPending, isError, error, refetch, isFetching } = useApiTokens();
  const repositories = useRepositories();
  const createToken = useCreateApiToken();
  const revokeToken = useRevokeApiToken();
  const [name, setName] = useState("");
  const [expiresAt, setExpiresAt] = useState("");
  const [scopeRead, setScopeRead] = useState(false);
  const [scopeWrite, setScopeWrite] = useState(false);
  const [repositoryIds, setRepositoryIds] = useState<string[]>([]);
  const [createdSecret, setCreatedSecret] = useState<string | null>(null);

  function selectedScopes(): string[] | undefined {
    const scopes: string[] = [];
    if (scopeRead) {
      scopes.push("read");
    }
    if (scopeWrite) {
      scopes.push("write");
    }
    return scopes.length === 0 ? undefined : scopes;
  }

  async function onCreate(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    try {
      const result = await createToken.mutateAsync({
        name: name.trim(),
        expires_at: expiresAt.trim() ? new Date(expiresAt).toISOString() : undefined,
        scopes: selectedScopes(),
        repository_ids: repositoryIds.length === 0 ? undefined : repositoryIds,
      });
      setCreatedSecret(result.token);
      setName("");
      setExpiresAt("");
      setScopeRead(false);
      setScopeWrite(false);
      setRepositoryIds([]);
      toast.success(t("security.created"));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("security.createFailed"));
    }
  }

  async function onRevoke(tokenId: string, tokenName: string) {
    try {
      await revokeToken.mutateAsync(tokenId);
      toast.success(t("security.revoked", { name: tokenName }));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("security.revokeFailed"));
    }
  }

  return (
    <div>
      <PageHeader title={t("security.title")} description={t("security.description")} />

      <form onSubmit={(event) => void onCreate(event)} className="mb-8 space-y-4">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-end">
          <div className="w-full space-y-2 sm:max-w-sm">
            <Label htmlFor="token-name">{t("security.tokenName")}</Label>
            <Input
              id="token-name"
              placeholder="cargo-publish"
              value={name}
              onChange={(event) => setName(event.target.value)}
              required
            />
          </div>
          <div className="w-full space-y-2 sm:max-w-xs">
            <Label htmlFor="token-expires">{t("security.expiresAt")}</Label>
            <Input
              id="token-expires"
              type="datetime-local"
              value={expiresAt}
              onChange={(event) => setExpiresAt(event.target.value)}
            />
          </div>
          <Button type="submit" disabled={createToken.isPending || name.trim().length === 0}>
            <KeyRound />
            {createToken.isPending ? t("security.creating") : t("security.createToken")}
          </Button>
        </div>
        <fieldset className="space-y-2">
          <legend className="text-sm font-medium">{t("security.scopes")}</legend>
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={scopeRead}
              onChange={(event) => setScopeRead(event.target.checked)}
            />
            {t("security.scopeRead")}
          </label>
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={scopeWrite}
              onChange={(event) => setScopeWrite(event.target.checked)}
            />
            {t("security.scopeWrite")}
          </label>
          <p className="text-xs text-muted-foreground">{t("security.scopeHint")}</p>
        </fieldset>
        <fieldset className="space-y-2">
          <legend className="text-sm font-medium">{t("security.repositories")}</legend>
          {repositories.data && repositories.data.length > 0 ? (
            <div className="max-h-48 space-y-2 overflow-y-auto rounded-md border border-border p-3">
              {repositories.data.map((repository) => (
                <label key={repository.id} className="flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={repositoryIds.includes(repository.id)}
                    onChange={(event) => {
                      setRepositoryIds((current) =>
                        event.target.checked
                          ? [...current, repository.id]
                          : current.filter((id) => id !== repository.id),
                      );
                    }}
                  />
                  <span className="font-medium">{repository.name}</span>
                  <span className="text-muted-foreground">{repository.ecosystem}</span>
                </label>
              ))}
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">{t("security.repositoryEmpty")}</p>
          )}
          <p className="text-xs text-muted-foreground">{t("security.repositoryHint")}</p>
        </fieldset>
      </form>

      {isPending ? (
        <div className="space-y-2">
          {Array.from({ length: 3 }, (_, index) => (
            <Skeleton key={index} className="h-12 w-full rounded-lg" />
          ))}
        </div>
      ) : null}

      {isError ? (
        <Alert variant="destructive">
          <AlertCircle />
          <AlertTitle>{t("security.loadFailed")}</AlertTitle>
          <AlertDescription className="flex items-center justify-between gap-4">
            <span>{error.message}</span>
            <Button size="sm" variant="outline" onClick={() => void refetch()}>
              <RefreshCw className={isFetching ? "animate-spin" : ""} />
              {t("common.retry")}
            </Button>
          </AlertDescription>
        </Alert>
      ) : null}

      {data && data.length === 0 ? (
        <div className="rounded-lg border border-dashed border-border py-16 text-center">
          <p className="font-medium text-foreground">{t("security.emptyTitle")}</p>
          <p className="mt-1 text-sm text-muted-foreground">{t("security.emptyBody")}</p>
        </div>
      ) : null}

      {data && data.length > 0 ? (
        <div className="overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("common.name")}</TableHead>
                <TableHead>{t("security.prefix")}</TableHead>
                <TableHead>{t("security.createdAt")}</TableHead>
                <TableHead>{t("security.expiresAt")}</TableHead>
                <TableHead>{t("security.scopes")}</TableHead>
                <TableHead>{t("security.repositories")}</TableHead>
                <TableHead className="text-right">{t("common.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.map((token) => (
                <TableRow key={token.id}>
                  <TableCell className="font-medium">{token.name}</TableCell>
                  <TableCell className="font-mono text-xs text-muted-foreground">
                    {token.prefix}…
                  </TableCell>
                  <TableCell className="text-sm text-muted-foreground">
                    {formatCreatedAt(token.created_at)}
                  </TableCell>
                  <TableCell className="text-sm text-muted-foreground">
                    {token.expires_at ? formatCreatedAt(token.expires_at) : t("security.never")}
                  </TableCell>
                  <TableCell className="text-sm text-muted-foreground">
                    {formatScopes(token.scopes, t)}
                  </TableCell>
                  <TableCell className="text-sm text-muted-foreground">
                    {formatRepositories(token.repository_ids, repositories.data, t)}
                  </TableCell>
                  <TableCell className="text-right">
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={revokeToken.isPending}
                      onClick={() => void onRevoke(token.id, token.name)}
                    >
                      <Trash2 />
                      {t("security.revoke")}
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      ) : null}

      <Dialog open={createdSecret !== null} onOpenChange={(open) => !open && setCreatedSecret(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("security.copyNow")}</DialogTitle>
            <DialogDescription>{t("security.copyNowHint")}</DialogDescription>
          </DialogHeader>
          <div className="flex items-center gap-2">
            <Input readOnly value={createdSecret ?? ""} className="font-mono text-xs" />
            <Button
              type="button"
              variant="outline"
              size="icon"
              aria-label={t("security.copyToken")}
              onClick={() => {
                if (createdSecret) {
                  void navigator.clipboard.writeText(createdSecret);
                  toast.success(t("security.tokenCopied"));
                }
              }}
            >
              <Copy />
            </Button>
          </div>
          <DialogFooter>
            <Button type="button" onClick={() => setCreatedSecret(null)}>
              {t("security.saved")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function formatCreatedAt(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleString();
}

function formatRepositories(
  ids: string[],
  repositories: { id: string; name: string }[] | undefined,
  t: (key: string) => string,
): string {
  if (ids.length === 0) {
    return t("security.repositoryUnrestricted");
  }
  return ids
    .map((id) => repositories?.find((repository) => repository.id === id)?.name ?? id)
    .join(", ");
}

function formatScopes(scopes: string[], t: (key: string) => string): string {
  if (scopes.length === 0) {
    return t("security.scopeUnrestricted");
  }
  return scopes
    .map((scope) => {
      if (scope === "read") {
        return t("security.scopeRead");
      }
      if (scope === "write") {
        return t("security.scopeWrite");
      }
      return scope;
    })
    .join(", ");
}
