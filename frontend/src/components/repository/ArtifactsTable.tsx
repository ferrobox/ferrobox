import { useMemo, useState } from "react";
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
  Trash2,
} from "lucide-react";
import { toast } from "sonner";

import { ApiError, downloadArtifact } from "@/api/client";
import type { ArtifactResponse } from "@/api/generated/ArtifactResponse";
import type { AssayResponse } from "@/api/generated/AssayResponse";
import type { PackageEcosystemDto } from "@/api/generated/PackageEcosystemDto";
import { useDeleteArtifact, useRepositoryArtifacts, useRepositoryAssays, useSetYanked } from "@/api/queries";
import { AssayDialog } from "@/components/assay/AssayDialog";
import { AssayCountPills } from "@/components/assay/SeverityBadges";
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

function displayName(artifact: ArtifactResponse): string {
  return artifact.name ?? "Artefacto sin índice";
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

function groupArtifacts(
  data: ArtifactResponse[],
  ecosystem: PackageEcosystemDto,
): PackageGroup[] {
  const byName = new Map<string, ArtifactResponse[]>();
  for (const artifact of data) {
    if (artifact.name === "_blob") {
      continue;
    }
    const key = artifact.name ?? artifact.id;
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
  const { data, isPending, isError, error, refetch, isFetching } =
    useRepositoryArtifacts(repositoryId);
  const { data: assays } = useRepositoryAssays(repositoryId);
  const deleteArtifact = useDeleteArtifact(repositoryId);
  const setYanked = useSetYanked(repositoryId);
  const canYank =
    canWrite &&
    kind === "forge" &&
    (ecosystem === "cargo" ||
      ecosystem === "npm" ||
      ecosystem === "pypi" ||
      ecosystem === "oci" ||
      ecosystem === "helm" ||
      ecosystem === "conan");
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
      toast.success("Artefacto eliminado");
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo eliminar el artefacto");
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
          ? `${artifact.name} ${label} marcado como yanked`
          : `${artifact.name} ${label} restaurado`,
      );
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo actualizar el yank");
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

  function onDownload(artifact: ArtifactResponse) {
    void downloadArtifact(artifact.id, downloadName(artifact, ecosystem)).catch(
      (err: unknown) => {
        toast.error(
          err instanceof ApiError ? err.message : "No se pudo descargar el artefacto",
        );
      },
    );
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
        <AlertTitle>No se pudieron cargar los artefactos</AlertTitle>
        <AlertDescription className="flex items-center justify-between gap-4">
          <span>{error.message}</span>
          <Button size="sm" variant="outline" onClick={() => void refetch()}>
            <RefreshCw className={isFetching ? "animate-spin" : ""} />
            Reintentar
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
        <p className="font-medium text-foreground">Este repositorio está vacío</p>
        <p className="max-w-sm text-sm text-muted-foreground">
          {kind === "alloy" ? (
            "Los paquetes aparecen cuando existen en los Forges o Mirrors miembros. Publica en un Forge miembro."
          ) : kind === "mirror" ? (
            ecosystem === "npm"
              ? "Los paquetes se cachean la primera vez que npm install los resuelve contra este Mirror."
              : ecosystem === "pypi"
                ? "Los paquetes se cachean la primera vez que pip install los resuelve contra este Mirror."
                : ecosystem === "oci"
                  ? "Las imágenes se cachean la primera vez que docker pull las resuelve contra este Mirror."
                  : ecosystem === "helm"
                    ? "Los charts se cachean la primera vez que helm pull los resuelve contra este Mirror."
                    : ecosystem === "conan"
                      ? "Los paquetes se cachean la primera vez que conan install los resuelve contra este Mirror."
                      : "Los paquetes se cachean la primera vez que cargo los resuelve contra este Mirror."
          ) : ecosystem === "npm" ? (
            <>
              Publica un paquete con <code className="font-mono">npm publish</code> apuntando a
              este registro.
            </>
          ) : ecosystem === "pypi" ? (
            <>
              Publica un paquete con <code className="font-mono">twine upload</code> apuntando a
              este registro.
            </>
          ) : ecosystem === "oci" ? (
            <>
              Publica una imagen con <code className="font-mono">docker push</code> apuntando a este
              registro.
            </>
          ) : ecosystem === "helm" ? (
            <>
              Publica un chart con <code className="font-mono">helm push</code> apuntando a este
              registro.
            </>
          ) : ecosystem === "conan" ? (
            <>
              Publica un paquete con <code className="font-mono">conan upload</code> apuntando a
              este registro.
            </>
          ) : (
            <>
              Publica un crate con <code className="font-mono">cargo publish --registry ferrobox</code>{" "}
              o sube un binario genérico.
            </>
          )}
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
                  {displayName(latest)}
                </span>
                <span className="block text-xs text-muted-foreground">
                  {group.versions.length === 1
                    ? "1 versión"
                    : `${group.versions.length} versiones`}
                  {yankedCount > 0
                    ? ` · ${yankedCount === 1 ? "1 yanked" : `${yankedCount} yanked`}`
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
  const representative = bucket.artifacts[0];
  if (!representative) {
    return null;
  }
  const yanked = bucket.artifacts.some((artifact) => artifact.yanked);
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
                {truncateMiddle(representative.id)}
              </span>
            )}
            {yanked ? (
              <Badge variant="outline" className="border-destructive/40 text-destructive">
                Yanked
              </Badge>
            ) : null}
            <span className="text-xs text-muted-foreground">
              {nested
                ? `${bucket.artifacts.length} ficheros · ${formatBytes(totalBytes)}`
                : formatBytes(representative.size_bytes)}
            </span>
            {memberName ? (
              <NavLink
                to={`/repositories/${representative.repository_id}`}
                className="text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
              >
                {memberName}
              </NavLink>
            ) : null}
            {assay ? <AssayCountPills counts={assay.counts} /> : null}
          </span>
          {nested ? null : (
            <span className="mt-0.5 block font-mono text-[11px] text-muted-foreground">
              sha256:{truncateMiddle(representative.checksum, 6)}
            </span>
          )}
        </span>
        <div className="flex shrink-0 gap-1">
          {nested ? null : (
            <Button variant="ghost" size="sm" onClick={() => onDownload(representative)}>
              <Download />
              Descargar
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
              title={
                yanked
                  ? "Vuelve a ofrecer esta versión en resoluciones nuevas"
                  : "Deja de usarse en resoluciones nuevas; sigue descargable si ya está fijado"
              }
              onClick={() => void onSetYanked(representative, !yanked)}
            >
              {yanked ? <RotateCcw /> : <Ban />}
              {yanked ? "Restaurar" : "Yank"}
            </Button>
          ) : null}
          {nested || !canWrite || kind === "alloy" ? null : (
            <ConfirmDeleteDialog
              title={`Eliminar ${downloadName(representative, ecosystem)}`}
              description="Se borrarán el objeto almacenado, los metadatos y la entrada de índice asociada. Esta acción no se puede deshacer."
              pending={deletePending}
              onConfirm={() => onDelete(representative.id)}
              trigger={
                <Button variant="ghost" size="sm">
                  <Trash2 />
                  Eliminar
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
                  {formatBytes(artifact.size_bytes)} · sha256:
                  {truncateMiddle(artifact.checksum, 6)}
                </span>
              </span>
              <div className="flex shrink-0 gap-1">
                <Button variant="ghost" size="sm" onClick={() => onDownload(artifact)}>
                  <Download />
                  Descargar
                </Button>
                {canWrite && kind !== "alloy" ? (
                  <ConfirmDeleteDialog
                    title={`Eliminar ${downloadName(artifact, ecosystem)}`}
                    description="Se borrará este fichero del almacenamiento. El resto de la versión no se modifica. Esta acción no se puede deshacer."
                    pending={deletePending}
                    onConfirm={() => onDelete(artifact.id)}
                    trigger={
                      <Button variant="ghost" size="sm">
                        <Trash2 />
                        Eliminar
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
