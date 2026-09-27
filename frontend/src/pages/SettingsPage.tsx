import { type FormEvent, useState } from "react";
import { AlertCircle, Check, Copy, KeyRound, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useChangePassword, useMyGroups, useSettings, useStorage } from "@/api/queries";
import { HealthIndicator } from "@/components/layout/HealthIndicator";
import { useAuth } from "@/auth/AuthProvider";
import { passwordMeetsPolicy } from "@/auth/passwordPolicy";
import { formatBytes } from "@/lib/format";
import { canManageUsers, roleLabel } from "@/auth/roles";
import { GarbageCollectionCard } from "@/components/cleanup/GarbageCollectionCard";
import { PageHeader } from "@/components/layout/PageHeader";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";

export function SettingsPage() {
  const { user } = useAuth();
  const { t } = useTranslation();
  const { data: settings, isPending } = useSettings();
  const { data: storage } = useStorage();
  const changePassword = useChangePassword();

  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [copiedUrl, setCopiedUrl] = useState(false);

  async function onChangePassword(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();

    if (newPassword !== confirmPassword) {
      toast.error(t("password.mismatch"));
      return;
    }

    if (!passwordMeetsPolicy(newPassword)) {
      toast.error(t("password.policy"));
      return;
    }

    try {
      await changePassword.mutateAsync({
        current_password: currentPassword,
        new_password: newPassword,
      });
      setCurrentPassword("");
      setNewPassword("");
      setConfirmPassword("");
      toast.success(t("settings.passwordUpdated"));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("settings.passwordFailed"));
    }
  }

  async function copyPublicUrl() {
    if (!settings) {
      return;
    }
    await navigator.clipboard.writeText(settings.public_base_url);
    setCopiedUrl(true);
    window.setTimeout(() => setCopiedUrl(false), 1500);
    toast.success(t("settings.urlCopied"));
  }

  const canSubmit =
    currentPassword.length > 0 &&
    newPassword.length > 0 &&
    confirmPassword.length > 0 &&
    !changePassword.isPending;

  return (
    <div>
      <PageHeader title={t("settings.title")} description={t("settings.description")} />

      <div className="grid gap-6 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle>{t("settings.account")}</CardTitle>
            <CardDescription>{t("settings.accountHint")}</CardDescription>
          </CardHeader>
          <CardContent className="space-y-6">
            <dl className="grid gap-3 text-sm sm:grid-cols-2">
              <div>
                <dt className="text-muted-foreground">{t("settings.username")}</dt>
                <dd className="mt-1 font-medium text-foreground">{user?.username ?? "—"}</dd>
              </div>
              <div>
                <dt className="text-muted-foreground">{t("settings.email")}</dt>
                <dd className="mt-1 font-medium text-foreground">{user?.email ?? "—"}</dd>
              </div>
              <div>
                <dt className="text-muted-foreground">{t("settings.role")}</dt>
                <dd className="mt-1">
                  {user ? <Badge variant="outline">{roleLabel(user.role)}</Badge> : "—"}
                </dd>
              </div>
              {user?.sso ? (
                <div>
                  <dt className="text-muted-foreground">{t("settings.sso")}</dt>
                  <dd className="mt-1">
                    <Badge variant="secondary">{t("settings.ssoLinked")}</Badge>
                  </dd>
                </div>
              ) : null}
            </dl>

            <form onSubmit={(event) => void onChangePassword(event)} className="space-y-4">
              <div className="space-y-2">
                <Label htmlFor="current-password">{t("settings.currentPassword")}</Label>
                <Input
                  id="current-password"
                  type="password"
                  autoComplete="current-password"
                  value={currentPassword}
                  onChange={(event) => setCurrentPassword(event.target.value)}
                  required
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="new-password">{t("settings.newPassword")}</Label>
                <Input
                  id="new-password"
                  type="password"
                  autoComplete="new-password"
                  value={newPassword}
                  onChange={(event) => setNewPassword(event.target.value)}
                  required
                />
                <p className="text-xs text-muted-foreground">{t("password.policy")}</p>
              </div>
              <div className="space-y-2">
                <Label htmlFor="confirm-password">{t("settings.confirmPassword")}</Label>
                <Input
                  id="confirm-password"
                  type="password"
                  autoComplete="new-password"
                  value={confirmPassword}
                  onChange={(event) => setConfirmPassword(event.target.value)}
                  required
                />
              </div>
              <Button type="submit" disabled={!canSubmit}>
                <KeyRound />
                {changePassword.isPending ? t("common.saving") : t("settings.changePassword")}
              </Button>
              <p className="text-xs text-muted-foreground">{t("settings.forgotAdmin")}</p>
            </form>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>{t("settings.instance")}</CardTitle>
            <CardDescription>{t("settings.instanceHint")}</CardDescription>
          </CardHeader>
          <CardContent className="space-y-5">
            {isPending ? (
              <div className="space-y-2">
                <Skeleton className="h-10 w-full rounded-md" />
                <Skeleton className="h-6 w-32 rounded-md" />
              </div>
            ) : (
              <>
                <div className="space-y-2">
                  <Label htmlFor="public-base-url">{t("settings.publicUrl")}</Label>
                  <div className="flex items-center gap-2">
                    <Input
                      id="public-base-url"
                      readOnly
                      value={settings?.public_base_url ?? ""}
                      className="font-mono text-xs"
                    />
                    <Button
                      type="button"
                      variant="outline"
                      size="icon"
                      aria-label={t("settings.copyPublicUrl")}
                      onClick={() => void copyPublicUrl()}
                    >
                      {copiedUrl ? <Check /> : <Copy />}
                    </Button>
                  </div>
                </div>
                <div>
                  <p className="text-sm text-muted-foreground">{t("settings.serverVersion")}</p>
                  <p className="mt-1 font-mono text-sm text-foreground">
                    {settings?.version ?? "—"}
                  </p>
                </div>
                <div>
                  <p className="text-sm text-muted-foreground">{t("storage.title")}</p>
                  <p className="mt-1 text-sm text-foreground">
                    {storage
                      ? t("storage.usedOfInstance", { used: formatBytes(storage.used_bytes) })
                      : "—"}
                  </p>
                  <p className="mt-1 text-xs text-muted-foreground">{t("storage.usedHint")}</p>
                </div>
                <div>
                  <p className="text-sm text-muted-foreground">{t("health.title")}</p>
                  <div className="mt-1">
                    <HealthIndicator />
                  </div>
                </div>
                <div>
                  <p className="text-sm text-muted-foreground">{t("settings.sso")}</p>
                  {settings?.oidc_enabled ? (
                    <p className="mt-1 font-mono text-sm break-all text-foreground">
                      {settings.oidc_issuer}
                    </p>
                  ) : (
                    <p className="mt-1 text-sm text-muted-foreground">{t("settings.ssoOff")}</p>
                  )}
                </div>
              </>
            )}
          </CardContent>
        </Card>
        <div className="lg:col-span-2">
          <MembershipsCard />
        </div>
        <GarbageCollectionCard />
      </div>
    </div>
  );
}

function MembershipsCard() {
  const { user } = useAuth();
  const { t } = useTranslation();
  const { data, isPending, isError, error, refetch, isFetching } = useMyGroups();
  const admin = canManageUsers(user?.role);

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("settings.groups")}</CardTitle>
        <CardDescription>{t("settings.groupsHint")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {isPending ? (
          <div className="space-y-2">
            <Skeleton className="h-10 w-full rounded-md" />
            <Skeleton className="h-10 w-2/3 rounded-md" />
          </div>
        ) : null}

        {isError ? (
          <Alert variant="destructive">
            <AlertCircle />
            <AlertTitle>{t("settings.groupsFailed")}</AlertTitle>
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
          <p className="text-sm text-muted-foreground">
            {admin ? t("settings.noGroupsAdmin") : t("settings.noGroupsUser")}
          </p>
        ) : null}

        {data && data.length > 0 ? (
          <ul className="space-y-3">
            {data.map((group) => (
              <li key={group.id} className="rounded-md border border-border px-3 py-2">
                <p className="font-medium text-foreground">{group.name}</p>
                {group.repositories.length === 0 ? (
                  <p className="mt-1 text-sm text-muted-foreground">
                    {t("settings.groupNoRepos")}
                  </p>
                ) : (
                  <ul className="mt-2 space-y-1">
                    {group.repositories.map((grant) => (
                      <li
                        key={grant.repository_id}
                        className="flex items-center justify-between gap-2 text-sm"
                      >
                        <span className="min-w-0 truncate">{grant.repository_name}</span>
                        <Badge variant="outline">{roleLabel(grant.role)}</Badge>
                      </li>
                    ))}
                  </ul>
                )}
              </li>
            ))}
          </ul>
        ) : null}

        {admin ? (
          <Button variant="outline" size="sm" asChild>
            <Link to="/groups">{t("settings.manageGroups")}</Link>
          </Button>
        ) : null}
      </CardContent>
    </Card>
  );
}
