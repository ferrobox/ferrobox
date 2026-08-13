import { AlertCircle, ChevronRight, RefreshCw, Trash2 } from "lucide-react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useDeleteRepository, useRepository } from "@/api/queries";
import { useAuth } from "@/auth/AuthProvider";
import { canWriteArtifacts } from "@/auth/roles";
import { ArtifactsTable } from "@/components/repository/ArtifactsTable";
import { CargoRegistryPanel } from "@/components/repository/CargoRegistryPanel";
import { ConfirmDeleteDialog } from "@/components/repository/ConfirmDeleteDialog";
import { EcosystemBadge } from "@/components/repository/EcosystemBadge";
import { RepositoryKindBadge } from "@/components/repository/RepositoryKindBadge";
import { UploadArtifactButton } from "@/components/repository/UploadArtifactButton";
import { PageHeader } from "@/components/layout/PageHeader";
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
        <Skeleton className="h-20 w-full rounded-lg" />
        <Skeleton className="h-64 w-full rounded-lg" />
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

  return (
    <div>
      <PageHeader
        breadcrumb={
          <div className="flex items-center gap-1.5 text-sm text-muted-foreground">
            <Link to="/repositories" className="hover:text-foreground hover:underline">
              Repositorios
            </Link>
            <ChevronRight className="size-3.5" />
            <span className="text-foreground">{repository.name}</span>
          </div>
        }
        title={repository.name}
        actions={
          canWrite ? (
            <div className="flex items-center gap-2">
              <UploadArtifactButton repositoryId={repositoryId} />
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
            </div>
          ) : undefined
        }
      />

      <div className="mb-6 flex flex-wrap items-center gap-2">
        <EcosystemBadge ecosystem={repository.ecosystem} />
        <RepositoryKindBadge kind={repository.kind} />
        <span className="font-mono text-xs text-muted-foreground">{repository.id}</span>
      </div>

      {repository.ecosystem === "cargo" ? (
        <section className="mb-8">
          <h2 className="mb-3 text-sm font-semibold tracking-wide text-foreground uppercase">
            Registro de Cargo
          </h2>
          <CargoRegistryPanel repositoryId={repositoryId} />
        </section>
      ) : null}

      <section>
        <h2 className="mb-3 text-sm font-semibold tracking-wide text-foreground uppercase">
          Artefactos
        </h2>
        <ArtifactsTable repositoryId={repositoryId} />
      </section>
    </div>
  );
}
