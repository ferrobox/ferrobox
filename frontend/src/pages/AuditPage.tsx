import { AlertCircle, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Navigate } from "react-router-dom";

import { useAuditEvents } from "@/api/queries";
import { useAuth } from "@/auth/AuthProvider";
import { canManageUsers } from "@/auth/roles";
import { PageHeader } from "@/components/layout/PageHeader";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

const ACTION_KEYS = [
  "user.created",
  "user.deleted",
  "user.role_changed",
  "user.password_reset",
  "user.password_changed",
  "user.sso_signed_in",
  "token.created",
  "token.revoked",
  "group.created",
  "group.deleted",
  "group.members_changed",
  "group.repositories_changed",
  "repository.created",
  "repository.deleted",
  "repository.members_changed",
  "artifact.published",
  "artifact.deleted",
  "package.published",
  "package.yanked",
  "package.unyanked",
  "package.promoted",
  "admission.policy_changed",
  "retention.policy_changed",
  "retention.applied",
  "retention.gc",
  "quota.changed",
  "webhook.created",
  "webhook.updated",
  "webhook.deleted",
] as const;

function formatWhen(value: string, locale: string): string {
  const parsed = Date.parse(value);
  if (Number.isNaN(parsed)) {
    return value;
  }
  return new Intl.DateTimeFormat(locale.startsWith("es") ? "es" : "en", {
    dateStyle: "short",
    timeStyle: "medium",
  }).format(parsed);
}

export function AuditPage() {
  const { t, i18n } = useTranslation();
  const { user } = useAuth();
  const { data, isPending, isError, error, refetch, isFetching } = useAuditEvents(
    canManageUsers(user?.role),
  );

  if (!canManageUsers(user?.role)) {
    return <Navigate to="/repositories" replace />;
  }

  return (
    <div>
      <PageHeader
        title={t("audit.title")}
        description={t("audit.description")}
        actions={
          <Button size="sm" variant="outline" onClick={() => void refetch()}>
            <RefreshCw className={isFetching ? "animate-spin" : ""} />
            {t("common.refresh")}
          </Button>
        }
      />

      {isPending ? (
        <div className="space-y-2">
          {Array.from({ length: 4 }, (_, index) => (
            <Skeleton key={index} className="h-12 w-full rounded-lg" />
          ))}
        </div>
      ) : null}

      {isError ? (
        <Alert variant="destructive">
          <AlertCircle />
          <AlertTitle>{t("audit.loadFailed")}</AlertTitle>
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
        <p className="text-sm text-muted-foreground">{t("audit.empty")}</p>
      ) : null}

      {data && data.length > 0 ? (
        <div className="overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("common.when")}</TableHead>
                <TableHead>{t("common.who")}</TableHead>
                <TableHead>{t("common.action")}</TableHead>
                <TableHead>{t("common.target")}</TableHead>
                <TableHead>{t("common.detail")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.map((event) => (
                <TableRow key={event.id}>
                  <TableCell className="whitespace-nowrap text-muted-foreground">
                    {formatWhen(event.created_at, i18n.resolvedLanguage ?? "en")}
                  </TableCell>
                  <TableCell className="font-medium">{event.actor}</TableCell>
                  <TableCell>
                    {ACTION_KEYS.includes(event.action as (typeof ACTION_KEYS)[number])
                      ? t(`audit.${event.action}`)
                      : event.action}
                  </TableCell>
                  <TableCell>
                    <span className="text-muted-foreground">{event.target_kind}</span>
                    {event.target ? (
                      <>
                        {" "}
                        <span className="font-medium">{event.target}</span>
                      </>
                    ) : null}
                  </TableCell>
                  <TableCell className="text-muted-foreground">{event.detail || "—"}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      ) : null}
    </div>
  );
}
