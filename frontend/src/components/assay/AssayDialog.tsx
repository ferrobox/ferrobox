import { Download, FlaskConical, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError, downloadAssaySbom } from "@/api/client";
import type { AssayResponse } from "@/api/generated/AssayResponse";
import type { PackageEcosystemDto } from "@/api/generated/PackageEcosystemDto";
import { useAssay, useRunAssay } from "@/api/queries";
import { useAuth } from "@/auth/AuthProvider";
import { canWriteArtifacts } from "@/auth/roles";
import { AssayCountPills, SeverityBadge } from "@/components/assay/SeverityBadges";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

function statusLabel(status: AssayResponse["status"], t: (key: string) => string): string {
  switch (status) {
    case "ready":
      return t("assays.ready");
    case "failed":
      return t("assays.failed");
    case "unsupported":
      return t("assays.unsupported");
    case "running":
      return t("assays.running");
    default:
      return status;
  }
}

export function AssayDialog({
  open,
  onOpenChange,
  repositoryId,
  ecosystem,
  name,
  version,
  canRerun,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  repositoryId: string;
  ecosystem: PackageEcosystemDto;
  name: string;
  version: string;
  canRerun?: boolean;
}) {
  const { t } = useTranslation();
  const { user } = useAuth();
  const canRerunAssay = canRerun ?? canWriteArtifacts(user?.role);
  const lookup = { ecosystem, name, version };
  const { data, isPending, isError, error, refetch, isFetching } = useAssay(
    repositoryId,
    lookup,
    open,
  );
  const rerun = useRunAssay(repositoryId);

  async function onDownload() {
    if (!data) {
      return;
    }
    try {
      await downloadAssaySbom(data.id, `${name.replaceAll("/", "_")}-${version}.cdx.json`);
      toast.success(t("assays.dialogDownloaded"));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("assays.dialogDownloadFailed"));
    }
  }

  async function onRerun() {
    try {
      await rerun.mutateAsync(lookup);
      toast.success(t("assays.dialogReran"));
      await refetch();
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("assays.dialogRerunFailed"));
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[90vh] max-w-[calc(100%-2rem)] flex-col overflow-hidden sm:max-w-4xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <FlaskConical className="size-4" />
            {t("assays.dialogTitle", { name, version })}
          </DialogTitle>
          <DialogDescription>{t("assays.dialogHint")}</DialogDescription>
        </DialogHeader>

        {isPending ? (
          <div className="space-y-2">
            <Skeleton className="h-10 w-full rounded-md" />
            <Skeleton className="h-48 w-full rounded-md" />
          </div>
        ) : null}

        {isError ? (
          <p className="text-sm text-destructive">{error.message}</p>
        ) : null}

        {data ? (
          <AssayBody
            assay={data}
            canRerun={canRerunAssay}
            rerunPending={rerun.isPending || isFetching}
            onDownload={() => void onDownload()}
            onRerun={() => void onRerun()}
          />
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

function AssayBody({
  assay,
  canRerun,
  rerunPending,
  onDownload,
  onRerun,
}: {
  assay: AssayResponse;
  canRerun: boolean;
  rerunPending: boolean;
  onDownload: () => void;
  onRerun: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="space-y-1">
          <p className="text-sm text-muted-foreground">
            {t("assays.status", { status: statusLabel(assay.status, t) })}
            {assay.scanned_at
              ? ` · ${new Date(assay.scanned_at).toLocaleString()}`
              : null}
          </p>
          <AssayCountPills counts={assay.counts} />
          <LicenseChips licenses={uniqueLicenses(assay)} empty={t("assays.noLicense")} />
        </div>
        <div className="flex gap-2">
          <Button type="button" variant="outline" size="sm" onClick={onDownload}>
            <Download />
            {t("assays.downloadSbom")}
          </Button>
          {canRerun ? (
            <Button type="button" size="sm" disabled={rerunPending} onClick={onRerun}>
              <RefreshCw className={rerunPending ? "animate-spin" : ""} />
              {t("assays.repeat")}
            </Button>
          ) : null}
        </div>
      </div>

      {assay.error_message ? (
        <p className="rounded-md border border-border bg-muted/40 px-3 py-2 text-sm text-muted-foreground">
          {assay.error_message}
        </p>
      ) : null}

      <Tabs defaultValue="findings" className="min-h-0 flex-1">
        <TabsList>
          <TabsTrigger value="findings">
            {t("assays.impurities", { count: assay.findings.length })}
          </TabsTrigger>
          <TabsTrigger value="components">
            {t("assays.composition", { count: assay.components.length })}
          </TabsTrigger>
        </TabsList>
        <TabsContent value="findings" className="mt-3 min-h-0 overflow-y-auto">
          {assay.findings.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              {t("assays.noVulns")}
            </p>
          ) : (
            <ul className="space-y-3">
              {assay.findings.map((finding) => (
                <li
                  key={`${finding.vulnerability_id}-${finding.component_name}`}
                  className="rounded-md border border-border p-3"
                >
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="font-mono text-sm text-foreground">
                      {finding.vulnerability_id}
                    </span>
                    <SeverityBadge severity={finding.severity} />
                    {finding.fixed_version ? (
                      <span className="text-xs text-muted-foreground">
                        {t("assays.fixedIn", { version: finding.fixed_version })}
                      </span>
                    ) : null}
                  </div>
                  <p className="mt-1 text-sm text-foreground">{finding.title}</p>
                  <p className="mt-1 font-mono text-xs text-muted-foreground">
                    {finding.component_name}@{finding.component_version}
                    {finding.aliases.length > 0 ? ` · ${finding.aliases.join(", ")}` : ""}
                  </p>
                  {finding.details_url ? (
                    <a
                      href={finding.details_url}
                      target="_blank"
                      rel="noreferrer"
                      className="mt-2 inline-block text-xs text-primary underline-offset-4 hover:underline"
                    >
                      {t("assays.publicAdvisory")}
                    </a>
                  ) : null}
                </li>
              ))}
            </ul>
          )}
        </TabsContent>
        <TabsContent value="components" className="mt-3 min-h-0 overflow-y-auto">
          <ul className="divide-y divide-border rounded-md border border-border">
            {assay.components.map((component) => (
              <li
                key={`${component.kind}-${component.name}-${component.version}`}
                className="flex flex-wrap items-center justify-between gap-3 px-3 py-2 text-sm"
              >
                <span>
                  <span className="font-mono text-foreground">{component.name}</span>
                  <span className="ml-2 font-mono text-muted-foreground">{component.version}</span>
                </span>
                <span className="flex flex-wrap items-center justify-end gap-2">
                  <LicenseChips
                    licenses={(component.licenses ?? []).filter((license) => !isPlaceholderLicense(license))}
                    empty="—"
                  />
                  <span className="text-xs text-muted-foreground">
                    {component.kind === "root"
                      ? t("assays.kindRoot")
                      : component.kind === "transitive"
                        ? t("assays.kindTransitive")
                        : t("assays.kindDirect")}
                  </span>
                </span>
              </li>
            ))}
          </ul>
        </TabsContent>
      </Tabs>
    </div>
  );
}

function isPlaceholderLicense(license: string): boolean {
  const lower = license.toLowerCase();
  return lower.startsWith("<") || lower.includes("put the package license");
}

function uniqueLicenses(assay: AssayResponse): string[] {
  const seen = new Set<string>();
  const licenses: string[] = [];
  for (const component of assay.components) {
    for (const license of component.licenses ?? []) {
      if (isPlaceholderLicense(license)) {
        continue;
      }
      const key = license.toLowerCase();
      if (seen.has(key)) {
        continue;
      }
      seen.add(key);
      licenses.push(license);
    }
  }
  return licenses;
}

function LicenseChips({
  licenses,
  empty,
}: {
  licenses: readonly string[];
  empty: string;
}) {
  if (licenses.length === 0) {
    return <span className="text-xs text-muted-foreground">{empty}</span>;
  }
  return (
    <span className="flex flex-wrap gap-1">
      {licenses.map((license) => (
        <Badge key={license} variant="secondary">
          {license}
        </Badge>
      ))}
    </span>
  );
}
