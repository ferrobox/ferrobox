import { useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { NavLink } from "react-router-dom";
import {
  AlertCircle,
  Ban,
  ChevronDown,
  ChevronRight,
  Download,
  FileBox,
  FlaskConical,
  Package,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError, downloadArtifact, listRepositoryArtifacts } from "@/api/client";
import type { ArtifactResponse } from "@/api/generated/ArtifactResponse";
import type { AssayResponse } from "@/api/generated/AssayResponse";
import type { PackageEcosystemDto } from "@/api/generated/PackageEcosystemDto";
import {
  queryKeys,
  useDeleteArtifact,
  usePrefetchPackage,
  useRepositoryArtifacts,
  useRepositoryAssays,
  useSetYanked,
} from "@/api/queries";
import { AssayDialog } from "@/components/assay/AssayDialog";
import { ConfirmDeleteDialog } from "@/components/repository/ConfirmDeleteDialog";
import { PromotePackageDialog } from "@/components/repository/PromotePackageDialog";
import type { RepositoryStorageKind } from "@/components/repository/RepositoryKindBadge";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { formatBytes, truncateMiddle } from "@/lib/format";

type VersionBucket = {
  key: string;
  version: string | null;
  artifacts: ArtifactResponse[];
};

type PackageGroup = {
  name: string;
  versions: VersionBucket[];
};

function fallbackFilename(artifact: ArtifactResponse, ecosystem: PackageEcosystemDto): string {
  if (artifact.name && artifact.version) {
    const extension =
      ecosystem === "npm"
        ? "tgz"
        : ecosystem === "pypi"
          ? "tar.gz"
          : ecosystem === "oci" || ecosystem === "helm"
            ? "json"
            : ecosystem === "conan"
              ? "tgz"
              : ecosystem === "maven"
                ? "jar"
                : ecosystem === "nuget"
                  ? "nupkg"
                  : ecosystem === "go"
                    ? "zip"
                    : "crate";
    const base = artifact.name.includes("/")
      ? artifact.name.slice(artifact.name.lastIndexOf("/") + 1)
      : artifact.name;
    return `${base}-${artifact.version}.${extension}`;
  }
  return artifact.id;
}

function fileLabel(artifact: ArtifactResponse, ecosystem: PackageEcosystemDto): string {
  return artifact.filename ?? fallbackFilename(artifact, ecosystem);
}

function downloadName(artifact: ArtifactResponse, ecosystem: PackageEcosystemDto): string {
  const label = fileLabel(artifact, ecosystem);
  const slash = Math.max(label.lastIndexOf("/"), label.lastIndexOf("\\"));
  return slash >= 0 ? label.slice(slash + 1) : label;
}

function displayName(artifact: ArtifactResponse, unnamed: string): string {
  return artifact.name ?? artifact.filename ?? unnamed;
}

function isNugetPrerelease(version: string | null | undefined): boolean {
  if (!version) {
    return false;
  }
  const core = version.split("+", 1)[0] ?? version;
  return core.includes("-");
}

function versionLabel(version: string, ecosystem: PackageEcosystemDto): string {
  if (ecosystem === "cargo") {
    return `v${version}`;
  }
  if (ecosystem === "conan") {
    return version.replace(/@([^:]+):/, "@$1/");
  }
  return version;
}

function isHiddenOciReference(
  artifact: ArtifactResponse,
  ecosystem: PackageEcosystemDto,
): boolean {
  if (ecosystem !== "oci" && ecosystem !== "helm") {
    return artifact.name === "_blob";
  }
  const version = artifact.version ?? "";
  return (
    artifact.name === "_blob" ||
    version.startsWith("sha256:") ||
    version.endsWith(".sig") ||
    version.endsWith(".att") ||
    version.endsWith(".sbom")
  );
}

function groupArtifacts(
  data: ArtifactResponse[],
  ecosystem: PackageEcosystemDto,
): PackageGroup[] {
  const byName = new Map<string, ArtifactResponse[]>();
  for (const artifact of data) {
    if (isHiddenOciReference(artifact, ecosystem)) {
      continue;
    }
    const key = artifact.name ?? artifact.filename ?? artifact.id;
    const existing = byName.get(key) ?? [];
    existing.push(artifact);
    byName.set(key, existing);
  }

  return [...byName.entries()]
    .map(([name, artifacts]) => {
      const byVersion = new Map<string, ArtifactResponse[]>();
      for (const artifact of artifacts) {
        const key = artifact.version ?? artifact.id;
        const existing = byVersion.get(key) ?? [];
        existing.push(artifact);
        byVersion.set(key, existing);
      }
      const versions = [...byVersion.entries()]
        .map(([key, files]) => ({
          key,
          version: files[0]?.version ?? null,
          artifacts: files.slice().sort((left, right) =>
            fileLabel(left, ecosystem).localeCompare(fileLabel(right, ecosystem)),
          ),
        }))
        .sort((left, right) =>
          (right.version ?? "").localeCompare(left.version ?? "", undefined, {
            numeric: true,
          }),
        );
      return { name, versions };
    })
    .sort((left, right) => left.name.localeCompare(right.name));
}

export function ArtifactsTable({
  repositoryId,
  kind,
  ecosystem,
  canWrite,
  memberNames = {},
}: {
  repositoryId: string;
  kind: RepositoryStorageKind;
  ecosystem: PackageEcosystemDto;
  canWrite: boolean;
  memberNames?: Readonly<Record<string, string>>;
}) {
  const { t } = useTranslation();
  const { data, isPending, isError, error, refetch, isFetching } =
    useRepositoryArtifacts(repositoryId);
  const { data: assays } = useRepositoryAssays(repositoryId);
  const deleteArtifact = useDeleteArtifact(repositoryId);
  const setYanked = useSetYanked(repositoryId);
  const prefetch = usePrefetchPackage(repositoryId);
  const queryClient = useQueryClient();
  const canYank =
    canWrite &&
    kind === "forge" &&
    (ecosystem === "cargo" ||
      ecosystem === "npm" ||
      ecosystem === "pypi" ||
      ecosystem === "oci" ||
      ecosystem === "helm" ||
      ecosystem === "conan" ||
      ecosystem === "maven" ||
      ecosystem === "nuget" ||
      ecosystem === "go");
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());
  const [assayTarget, setAssayTarget] = useState<{
    repositoryId: string;
    name: string;
    version: string;
  } | null>(null);

  const assayByKey = useMemo(() => {
    const map = new Map<string, NonNullable<typeof assays>[number]>();
    for (const assay of assays ?? []) {
      map.set(`${assay.repository_id}:${assay.name}:${assay.version}`, assay);
    }
    return map;
  }, [assays]);

  const groups = useMemo(
    () => (data ? groupArtifacts(data, ecosystem) : []),
    [data, ecosystem],
  );

  async function onDelete(artifactId: string) {
    try {
      await deleteArtifact.mutateAsync(artifactId);
      toast.success(t("artifacts.deleted"));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("artifacts.deleteFailed"));
      throw err;
    }
  }

  async function onSetYanked(artifact: ArtifactResponse, yanked: boolean) {
    if (!artifact.name || !artifact.version) {
      return;
    }
    try {
      await setYanked.mutateAsync({
        name: artifact.name,
        version: artifact.version,
        yanked,
        ecosystem,
      });
      const label = versionLabel(artifact.version, ecosystem);
      toast.success(
        yanked
          ? t("artifacts.yanked", { name: artifact.name, version: label })
          : t("artifacts.restored", { name: artifact.name, version: label }),
      );
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("artifacts.yankFailed"));
    }
  }

  function toggleGroup(name: string) {
    setCollapsed((current) => {
      const next = new Set(current);
      if (next.has(name)) {
        next.delete(name);
      } else {
        next.add(name);
      }
      return next;
    });
  }

  async function onDownload(artifact: ArtifactResponse) {
    try {
      let target = artifact;
      if (!artifact.cached) {
        if (!artifact.name || !artifact.version) {
          throw new ApiError(400, t("artifacts.downloadFailed"));
        }
        await prefetch.mutateAsync({
          name: artifact.name,
          version: artifact.version,
        });
        const fresh = await queryClient.fetchQuery({
          queryKey: queryKeys.repositoryArtifacts(repositoryId),
          queryFn: () => listRepositoryArtifacts(repositoryId),
        });
        const cached = fresh.find(
          (item) =>
            item.cached &&
            item.name === artifact.name &&
            item.version === artifact.version &&
            item.repository_id === artifact.repository_id,
        );
        if (!cached) {
          toast.error(t("artifacts.downloadFailed"));
          return;
        }
        target = cached;
      }
      await downloadArtifact(target.id, downloadName(target, ecosystem));
    } catch (err: unknown) {
      toast.error(err instanceof ApiError ? err.message : t("artifacts.downloadFailed"));
    }
  }

  if (isPending) {
    return (
      <div className="space-y-2">
        {Array.from({ length: 3 }, (_, index) => (
          <Skeleton key={index} className="h-14 w-full rounded-xl" />
        ))}
      </div>
    );
  }

  if (isError) {
    return (
      <Alert variant="destructive">
        <AlertCircle />
        <AlertTitle>{t("artifacts.loadFailed")}</AlertTitle>
        <AlertDescription className="flex items-center justify-between gap-4">
          <span>{error.message}</span>
          <Button size="sm" variant="outline" onClick={() => void refetch()}>
            <RefreshCw className={isFetching ? "animate-spin" : ""} />
            {t("common.retry")}
          </Button>
        </AlertDescription>
      </Alert>
    );
  }

  if (!data || groups.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center gap-2 rounded-xl border border-dashed border-border py-16 text-center">
        <div className="flex size-12 items-center justify-center rounded-full bg-muted">
          <FileBox className="size-6 text-muted-foreground" />
        </div>
        <p className="font-medium text-foreground">{t("artifacts.empty")}</p>
        <p className="max-w-sm text-sm text-muted-foreground">
          {kind === "alloy"
            ? t("artifacts.emptyAlloy")
            : kind === "mirror"
              ? ecosystem === "npm"
                ? t("artifacts.emptyMirrorNpm")
                : ecosystem === "pypi"
                  ? t("artifacts.emptyMirrorPypi")
                  : ecosystem === "oci"
                    ? t("artifacts.emptyMirrorOci")
                    : ecosystem === "helm"
                      ? t("artifacts.emptyMirrorHelm")
                      : ecosystem === "conan"
                        ? t("artifacts.emptyMirrorConan")
                        : ecosystem === "maven"
                          ? t("artifacts.emptyMirrorMaven")
                          : ecosystem === "nuget"
                            ? t("artifacts.emptyMirrorNuget")
                            : ecosystem === "go"
                              ? t("artifacts.emptyMirrorGo")
                              : t("artifacts.emptyMirrorCargo")
              : ecosystem === "npm"
                ? t("artifacts.emptyForgeNpm")
                : ecosystem === "pypi"
                  ? t("artifacts.emptyForgePypi")
                  : ecosystem === "oci"
                    ? t("artifacts.emptyForgeOci")
                    : ecosystem === "helm"
                      ? t("artifacts.emptyForgeHelm")
                      : ecosystem === "conan"
                        ? t("artifacts.emptyForgeConan")
                        : ecosystem === "maven"
                          ? t("artifacts.emptyForgeMaven")
                          : ecosystem === "nuget"
                            ? t("artifacts.emptyForgeNuget")
                            : ecosystem === "go"
                              ? t("artifacts.emptyForgeGo")
                              : t("artifacts.emptyForgeCargo")}
        </p>
      </div>
    );
  }

  return (
    <>
    <div className="overflow-hidden rounded-xl border border-border bg-card">
      {groups.map((group) => {
        const isCollapsed = collapsed.has(group.name);
        const latest = group.versions[0]?.artifacts[0];
        if (!latest) {
          return null;
        }
        const yankedCount = group.versions.filter((bucket) =>
          bucket.artifacts.some((artifact) => artifact.yanked),
        ).length;
        const signedCount = group.versions.filter((bucket) =>
          bucket.artifacts.some((artifact) => artifact.signed),
        ).length;
        const verifiedCount = group.versions.filter((bucket) =>
          bucket.artifacts.some((artifact) => artifact.verified),
        ).length;

        return (
          <section key={group.name} className="border-b border-border last:border-b-0">
            <button
              type="button"
              onClick={() => toggleGroup(group.name)}
              className="flex w-full items-center gap-3 px-4 py-3 text-left hover:bg-muted/40"
            >
              {isCollapsed ? (
                <ChevronRight className="size-4 shrink-0 text-muted-foreground" />
              ) : (
                <ChevronDown className="size-4 shrink-0 text-muted-foreground" />
              )}
              <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-orange-500/15 text-orange-400">
                <Package className="size-4" />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block truncate font-medium text-foreground">
                  {displayName(latest, t("artifacts.unnamed"))}
                </span>
                <span className="block text-xs text-muted-foreground">
                  {group.versions.length === 1
                    ? t("artifacts.versionOne")
                    : t("artifacts.versionMany", { count: group.versions.length })}
                  {yankedCount > 0
                    ? ` · ${yankedCount === 1 ? t("artifacts.yankedOne") : t("artifacts.yankedMany", { count: yankedCount })}`
                    : null}
                  {signedCount > 0
                    ? ` · ${signedCount === 1 ? t("artifacts.signedOne") : t("artifacts.signedMany", { count: signedCount })}`
                    : null}
                  {verifiedCount > 0
                    ? ` · ${verifiedCount === 1 ? t("artifacts.verifiedOne") : t("artifacts.verifiedMany", { count: verifiedCount })}`
                    : null}
                </span>
              </span>
            </button>
            {isCollapsed ? null : (
              <ul className="border-t border-border bg-background/40">
                {group.versions.map((bucket) => (
                  <VersionRows
                    key={bucket.key}
                    bucket={bucket}
                    sourceRepositoryId={repositoryId}
                    kind={kind}
                    ecosystem={ecosystem}
                    memberNames={memberNames}
                    canWrite={canWrite}
                    canYank={canYank}
                    yankPending={setYanked.isPending}
                    deletePending={deleteArtifact.isPending}
                    onDownload={onDownload}
                    onSetYanked={onSetYanked}
                    onDelete={onDelete}
                    assayByKey={assayByKey}
                    onAssay={(artifact) => {
                      if (artifact.name && artifact.version) {
                        setAssayTarget({
                          repositoryId: artifact.repository_id,
                          name: artifact.name,
                          version: artifact.version,
                        });
                      }
                    }}
                  />
                ))}
              </ul>
            )}
          </section>
        );
      })}
    </div>
    {assayTarget ? (
      <AssayDialog
        open
        onOpenChange={(open) => {
          if (!open) {
            setAssayTarget(null);
          }
        }}
        repositoryId={assayTarget.repositoryId}
        ecosystem={ecosystem}
        name={assayTarget.name}
        version={assayTarget.version}
        canRerun={canWrite}
      />
    ) : null}
    </>
  );
}

function VersionRows({
  bucket,
  sourceRepositoryId,
  kind,
  ecosystem,
  memberNames,
  canWrite,
  canYank,
  yankPending,
  deletePending,
  onDownload,
  onSetYanked,
  onDelete,
  assayByKey,
  onAssay,
}: {
  bucket: VersionBucket;
  sourceRepositoryId: string;
  kind: RepositoryStorageKind;
  ecosystem: PackageEcosystemDto;
  memberNames: Readonly<Record<string, string>>;
  canWrite: boolean;
  canYank: boolean;
  yankPending: boolean;
  deletePending: boolean;
  onDownload: (artifact: ArtifactResponse) => void;
  onSetYanked: (artifact: ArtifactResponse, yanked: boolean) => void;
  onDelete: (artifactId: string) => Promise<void>;
  assayByKey: ReadonlyMap<string, AssayResponse>;
  onAssay: (artifact: ArtifactResponse) => void;
}) {
  const { t } = useTranslation();
  const representative = bucket.artifacts[0];
  if (!representative) {
    return null;
  }
  const yanked = bucket.artifacts.some((artifact) => artifact.yanked);
  const signed = bucket.artifacts.some((artifact) => artifact.signed);
  const verified = bucket.artifacts.some((artifact) => artifact.verified);
  const totalBytes = bucket.artifacts.reduce((sum, artifact) => sum + artifact.size_bytes, 0);
  const nested = bucket.artifacts.length > 1;
  const memberName =
    kind === "alloy"
      ? (memberNames[representative.repository_id] ?? representative.repository_id)
      : null;
  const assay =
    representative.name && representative.version
      ? assayByKey.get(
          `${representative.repository_id}:${representative.name}:${representative.version}`,
        )
      : undefined;

  return (
    <li className="border-l-2 border-l-orange-500/40">
      <div className="flex items-center gap-3 px-4 py-2.5 pl-14 hover:bg-muted/30">
        <span className="min-w-0 flex-1">
          <span className="flex flex-wrap items-center gap-2">
            {bucket.version ? (
              <Badge
                variant="outline"
                className={`font-mono ${yanked ? "text-muted-foreground line-through" : ""}`}
              >
                {versionLabel(bucket.version, ecosystem)}
              </Badge>
            ) : (
              <span className="font-mono text-xs text-muted-foreground">
                {representative.filename ?? truncateMiddle(representative.id)}
              </span>
            )}
            {ecosystem === "maven" && bucket.version?.endsWith("-SNAPSHOT") ? (
              <Badge
                variant="outline"
                className="border-amber-500/40 bg-amber-500/10 text-amber-800 dark:text-amber-300"
              >
                {t("artifacts.snapshot")}
              </Badge>
            ) : ecosystem === "maven" && bucket.version ? (
              <Badge variant="outline" className="border-slate-400/40 text-muted-foreground">
                {t("artifacts.release")}
              </Badge>
            ) : (ecosystem === "nuget" || ecosystem === "go") && isNugetPrerelease(bucket.version) ? (
              <Badge
                variant="outline"
                className="border-violet-500/40 bg-violet-500/10 text-violet-800 dark:text-violet-300"
              >
                {t("artifacts.prerelease")}
              </Badge>
            ) : (ecosystem === "nuget" || ecosystem === "go") && bucket.version ? (
              <Badge variant="outline" className="border-slate-400/40 text-muted-foreground">
                {t("artifacts.release")}
              </Badge>
            ) : null}
            {yanked ? (
              <Badge variant="outline" className="border-destructive/40 text-destructive">
                Yanked
              </Badge>
            ) : null}
            {verified ? (
              <Badge
                variant="outline"
                className="border-emerald-500/40 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
                title={t("artifacts.cosignTitle")}
              >
                <ShieldCheck />
                {t("artifacts.verified")}
              </Badge>
            ) : signed ? (
              <Badge
                variant="outline"
                className="border-emerald-500/40 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
                title={t("artifacts.signedTitle")}
              >
                <ShieldCheck />
                {t("artifacts.signed")}
              </Badge>
            ) : null}
            <span className="text-xs text-muted-foreground">
              {representative.cached
                ? nested
                  ? t("artifacts.files", {
                      count: bucket.artifacts.length,
                      size: formatBytes(totalBytes),
                    })
                  : formatBytes(representative.size_bytes)
                : t("artifacts.notDownloaded")}
            </span>
            {memberName ? (
              <NavLink
                to={`/repositories/${representative.repository_id}`}
                className="text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
              >
                {memberName}
              </NavLink>
            ) : null}
          </span>
          {nested || !representative.cached ? null : (
            <span className="mt-0.5 block font-mono text-[11px] text-muted-foreground">
              sha256:{truncateMiddle(representative.checksum, 6)}
            </span>
          )}
        </span>
        {assay?.status === "ready" ? (
          <button
            type="button"
            className="shrink-0 rounded-md border border-border bg-background px-2 py-1 font-mono text-xs text-foreground hover:bg-muted"
            onClick={() => onAssay(representative)}
            aria-label={t("artifacts.postureLabel", {
              critical: assay.counts.critical,
              high: assay.counts.high,
            })}
          >
            {t("artifacts.posture", {
              critical: assay.counts.critical,
              high: assay.counts.high,
            })}
          </button>
        ) : null}
        <div className="flex shrink-0 gap-1">
          {nested ? null : (
            <Button variant="ghost" size="sm" onClick={() => onDownload(representative)}>
              <Download />
              {t("artifacts.download")}
            </Button>
          )}
          {representative.name && representative.version ? (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onAssay(representative)}
            >
              <FlaskConical />
              Assay
            </Button>
          ) : null}
          {kind === "forge" ? (
            <PromotePackageDialog
              sourceRepositoryId={sourceRepositoryId}
              ecosystem={ecosystem}
              artifact={representative}
              versionLabel={
                bucket.version ? versionLabel(bucket.version, ecosystem) : representative.id
              }
            />
          ) : null}
          {canYank && representative.name && representative.version ? (
            <Button
              variant="ghost"
              size="sm"
              disabled={yankPending}
              title={yanked ? t("artifacts.unyankHint") : t("artifacts.yankHint")}
              onClick={() => void onSetYanked(representative, !yanked)}
            >
              {yanked ? <RotateCcw /> : <Ban />}
              {yanked ? t("artifacts.unyank") : t("artifacts.yank")}
            </Button>
          ) : null}
          {nested || !canWrite || kind === "alloy" || !representative.cached ? null : (
            <ConfirmDeleteDialog
              title={t("artifacts.deleteTitle", {
                name: downloadName(representative, ecosystem),
              })}
              description={t("artifacts.deleteObject")}
              pending={deletePending}
              onConfirm={() => onDelete(representative.id)}
              trigger={
                <Button variant="ghost" size="sm">
                  <Trash2 />
                  {t("common.delete")}
                </Button>
              }
            />
          )}
        </div>
      </div>
      {nested
        ? bucket.artifacts.map((artifact) => (
            <div
              key={artifact.id}
              className="flex items-center gap-3 border-t border-border/60 px-4 py-2 pl-20 hover:bg-muted/20"
            >
              <span className="min-w-0 flex-1">
                <span className="block truncate font-mono text-xs text-foreground">
                  {fileLabel(artifact, ecosystem)}
                </span>
                <span className="mt-0.5 block font-mono text-[11px] text-muted-foreground">
                  {artifact.cached
                    ? `${formatBytes(artifact.size_bytes)} · sha256:${truncateMiddle(artifact.checksum, 6)}`
                    : t("artifacts.notDownloaded")}
                </span>
              </span>
              <div className="flex shrink-0 gap-1">
                <Button variant="ghost" size="sm" onClick={() => onDownload(artifact)}>
                  <Download />
                  {t("artifacts.download")}
                </Button>
                {canWrite && kind !== "alloy" && artifact.cached ? (
                  <ConfirmDeleteDialog
                    title={t("artifacts.deleteTitle", {
                      name: downloadName(artifact, ecosystem),
                    })}
                    description={t("artifacts.deleteFile")}
                    pending={deletePending}
                    onConfirm={() => onDelete(artifact.id)}
                    trigger={
                      <Button variant="ghost" size="sm">
                        <Trash2 />
                        {t("common.delete")}
                      </Button>
                    }
                  />
                ) : null}
              </div>
            </div>
          ))
        : null}
    </li>
  );
}
