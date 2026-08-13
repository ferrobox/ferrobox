import { useState } from "react";
import { AlertCircle, Check, Copy, RefreshCw, Trash2 } from "lucide-react";
import { useNavigate, useParams } from "react-router-dom";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useDeleteRepository, useRepository } from "@/api/queries";
import { useAuth } from "@/auth/AuthProvider";
import { canWriteArtifacts } from "@/auth/roles";
import { ArtifactsTable } from "@/components/repository/ArtifactsTable";
import { ConfirmDeleteDialog } from "@/components/repository/ConfirmDeleteDialog";
import { EcosystemBadge, ecosystemMeta } from "@/components/repository/EcosystemBadge";
import { KIND_META, RepositoryKindBadge } from "@/components/repository/RepositoryKindBadge";
import { SetMeUpDialog } from "@/components/repository/SetMeUpDialog";
import { UploadArtifactButton } from "@/components/repository/UploadArtifactButton";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import NotFoundPage from "@/pages/NotFoundPage";

export function RepositoryDetailPage() {
  const { repositoryId } = useParams<{ repositoryId: string }>();

  if (!repositoryId) {
    return <NotFoundPage />;
  }

  return <RepositoryDetailContent repositoryId={repositoryId} />;
}

function RepositoryDetailContent({ repositoryId }: { repositoryId: string }) {
  const { user } = useAuth();
  const navigate = useNavigate();
  const { data: repository, isPending, isError, error, refetch, isFetching } =
    useRepository(repositoryId);
  const deleteRepository = useDeleteRepository();
  const canWrite = canWriteArtifacts(user?.role);
  const [copiedPath, setCopiedPath] = useState(false);

  async function onDeleteRepository() {
    try {
      await deleteRepository.mutateAsync(repositoryId);
      toast.success(`Repositorio «${repository?.name ?? repositoryId}» eliminado`);
      navigate("/repositories", { replace: true });
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo eliminar el repositorio");
      throw err;
    }
  }

  if (isPending) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-28 w-full rounded-xl" />
        <Skeleton className="h-64 w-full rounded-xl" />
      </div>
    );
  }

  if (isError) {
    if (error instanceof ApiError && error.status === 404) {
      return <NotFoundPage message="Ese repositorio no existe." />;
    }

    return (
      <Alert variant="destructive">
        <AlertCircle />
        <AlertTitle>No se pudo cargar el repositorio</AlertTitle>
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

  const kind = KIND_META[repository.kind.type];
  const eco = ecosystemMeta(repository.ecosystem);
  const EcoIcon = eco.icon;
  const path = `${repository.ecosystem}://${repository.name}/`;

  async function copyPath() {
    await navigator.clipboard.writeText(path);
    setCopiedPath(true);
    window.setTimeout(() => setCopiedPath(false), 1500);
  }

  return (
    <div className="space-y-8">
      <header className="overflow-hidden rounded-xl border border-border bg-card">
        <div className="flex items-start gap-4 p-6">
          <span
            className={`flex size-12 shrink-0 items-center justify-center rounded-xl ${eco.className}`}
          >
            <EcoIcon className="size-6" />
          </span>
          <div className="min-w-0 flex-1">
            <p className="text-[11px] font-semibold tracking-[0.2em] text-muted-foreground uppercase">
              {kind.label} · {kind.domain}
            </p>
            <h1 className="mt-1 truncate text-2xl font-semibold tracking-tight text-foreground">
              {repository.name}
            </h1>
            <div className="mt-3 flex flex-wrap items-center gap-2">
              <EcosystemBadge ecosystem={repository.ecosystem} />
              <RepositoryKindBadge kind={repository.kind} />
              <button
                type="button"
                onClick={() => void copyPath()}
                className="inline-flex items-center gap-1.5 rounded-md border border-border bg-background px-2 py-1 font-mono text-[11px] text-muted-foreground hover:text-foreground"
                title="Copiar ruta"
              >
                {copiedPath ? <Check className="size-3" /> : <Copy className="size-3" />}
                {path}
              </button>
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {repository.ecosystem === "cargo" ? (
              <SetMeUpDialog
                repositoryId={repositoryId}
                isMirror={repository.kind.type === "mirror"}
              />
            ) : null}
            {canWrite && repository.kind.type !== "mirror" ? (
              <UploadArtifactButton repositoryId={repositoryId} />
            ) : null}
            {canWrite ? (
              <ConfirmDeleteDialog
                title={`Eliminar «${repository.name}»`}
                description="Se borrarán el repositorio, sus artefactos, el índice de paquetes y los objetos almacenados. Esta acción no se puede deshacer."
                pending={deleteRepository.isPending}
                onConfirm={onDeleteRepository}
                trigger={
                  <Button variant="outline">
                    <Trash2 />
                    Eliminar
                  </Button>
                }
              />
            ) : null}
          </div>
        </div>
      </header>

      <section>
        <div className="mb-3 flex items-baseline justify-between gap-3">
          <h2 className="text-sm font-semibold tracking-wide text-foreground uppercase">
            Paquetes
          </h2>
          <p className="text-xs text-muted-foreground">
            Agrupados por nombre, con cada versión debajo.
          </p>
        </div>
        <ArtifactsTable repositoryId={repositoryId} />
      </section>
    </div>
  );
}
