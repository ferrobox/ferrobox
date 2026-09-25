import { useState } from "react";
import { AlertCircle, FlaskConical, Loader2, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { NavLink } from "react-router-dom";
import { toast } from "sonner";

import type { AssayResponse } from "@/api/generated/AssayResponse";
import { useAssays, useRerunAllAssays, useRepositories } from "@/api/queries";
import { useAuth } from "@/auth/AuthProvider";
import { canWriteArtifacts } from "@/auth/roles";
import { AssayDialog } from "@/components/assay/AssayDialog";
import { AssayCountPills } from "@/components/assay/SeverityBadges";
import { PageHeader } from "@/components/layout/PageHeader";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
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

function isPlaceholderLicense(license: string): boolean {
  const lower = license.toLowerCase();
  return lower.startsWith("<") || lower.includes("put the package license");
}

function AssayLicenseSummary({ assay }: { assay: AssayResponse }) {
  const { t } = useTranslation();
  const root = assay.components.find((component) => component.kind === "root");
  const rootLicenses = [...new Set((root?.licenses ?? []).filter((license) => !isPlaceholderLicense(license)))];
  const all = [
    ...new Set(
      assay.components
        .flatMap((component) => component.licenses ?? [])
        .filter((license) => !isPlaceholderLicense(license)),
    ),
  ];
  const shown = rootLicenses.length > 0 ? rootLicenses : all.slice(0, 2);
  const extra = rootLicenses.length > 0 ? 0 : Math.max(0, all.length - shown.length);

  if (shown.length === 0) {
    return <span className="text-xs text-muted-foreground">—</span>;
  }

  return (
    <span className="flex flex-wrap gap-1">
      {shown.map((license) => (
        <Badge key={license} variant="secondary">
          {license}
        </Badge>
      ))}
      {extra > 0 ? (
        <span className="text-xs text-muted-foreground">
          {t("assays.extraInComposition", { count: extra })}
        </span>
      ) : null}
    </span>
  );
}

function statusLabel(status: string, t: (key: string) => string): string {
  if (status === "ready") {
    return t("assays.ready");
  }
  if (status === "failed") {
    return t("assays.failed");
  }
  if (status === "unsupported") {
    return t("assays.unsupported");
  }
  if (status === "running") {
    return t("assays.running");
  }
  return status;
}

export function AssaysPage() {
  const { t } = useTranslation();
  const { user } = useAuth();
  const canWrite = canWriteArtifacts(user?.role);
  const { data, isPending, isError, error, refetch, isFetching } = useAssays();
  const { data: repositories } = useRepositories();
  const rerunAll = useRerunAllAssays();
  const names = new Map((repositories ?? []).map((repository) => [repository.id, repository.name]));
  const [openAssay, setOpenAssay] = useState<AssayResponse | null>(null);
  const batchRunning =
    rerunAll.isPending || (data?.some((assay) => assay.status === "running") ?? false);

  return (
    <div>
      <PageHeader
        title={t("assays.title")}
        description={t("assays.description")}
        actions={
          canWrite && data && data.length > 0 ? (
            <Button
              type="button"
              variant="outline"
              disabled={batchRunning}
              onClick={() => {
                void rerunAll.mutateAsync().then(
                  (result) => {
                    toast.success(
                      result.scheduled === 1
                        ? t("assays.rerunOne")
                        : t("assays.rerunMany", { count: result.scheduled }),
                    );
                  },
                  () => undefined,
                );
              }}
            >
              <RefreshCw className={batchRunning ? "animate-spin" : ""} />
              {batchRunning ? t("assays.rerunning") : t("assays.rerunAll")}
            </Button>
          ) : null
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
          <AlertTitle>{t("assays.loadFailed")}</AlertTitle>
          <AlertDescription className="flex items-center justify-between gap-4">
            <span>{error.message}</span>
            <Button size="sm" variant="outline" onClick={() => void refetch()}>
              <RefreshCw className={isFetching ? "animate-spin" : ""} />
              {t("common.retry")}
            </Button>
          </AlertDescription>
        </Alert>
      ) : null}

      {rerunAll.isError ? (
        <Alert variant="destructive" className="mb-4">
          <AlertCircle />
          <AlertTitle>{t("assays.rerunFailed")}</AlertTitle>
          <AlertDescription>{rerunAll.error.message}</AlertDescription>
        </Alert>
      ) : null}

      {data && data.length === 0 ? (
        <div className="rounded-lg border border-dashed border-border py-16 text-center">
          <FlaskConical className="mx-auto size-8 text-muted-foreground" />
          <p className="mt-3 font-medium text-foreground">{t("assays.emptyTitle")}</p>
          <p className="mt-1 text-sm text-muted-foreground">{t("assays.emptyBody")}</p>
        </div>
      ) : null}

      {data && data.length > 0 ? (
        <div className="overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("common.package")}</TableHead>
                <TableHead>{t("common.repository")}</TableHead>
                <TableHead>{t("common.status")}</TableHead>
                <TableHead>{t("common.licenses")}</TableHead>
                <TableHead>{t("common.findings")}</TableHead>
                <TableHead className="w-[1%]" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.map((assay) => (
                <TableRow key={assay.id}>
                  <TableCell>
                    <NavLink
                      to={`/repositories/${assay.repository_id}`}
                      className="font-mono text-sm text-foreground underline-offset-4 hover:underline"
                    >
                      {assay.name}@{assay.version}
                    </NavLink>
                    <p className="text-xs text-muted-foreground">{assay.ecosystem}</p>
                  </TableCell>
                  <TableCell className="text-sm">
                    {names.get(assay.repository_id) ?? assay.repository_id}
                  </TableCell>
                  <TableCell>
                    <Badge variant="outline" className="gap-1">
                      {assay.status === "running" ? (
                        <Loader2 className="size-3 animate-spin" />
                      ) : null}
                      {statusLabel(assay.status, t)}
                    </Badge>
                  </TableCell>
                  <TableCell className="whitespace-normal">
                    <AssayLicenseSummary assay={assay} />
                  </TableCell>
                  <TableCell>
                    <AssayCountPills counts={assay.counts} />
                  </TableCell>
                  <TableCell>
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      onClick={() => setOpenAssay(assay)}
                    >
                      {t("common.view")}
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      ) : null}

      {openAssay ? (
        <AssayDialog
          open
          onOpenChange={(open) => {
            if (!open) {
              setOpenAssay(null);
            }
          }}
          repositoryId={openAssay.repository_id}
          ecosystem={openAssay.ecosystem}
          name={openAssay.name}
          version={openAssay.version}
          canRerun={
            repositories?.find((repository) => repository.id === openAssay.repository_id)
              ?.access === "write"
          }
        />
      ) : null}
    </div>
  );
}
