import { useState } from "react";
import { AlertCircle, Check, Copy, RefreshCw, Trash2 } from "lucide-react";
import { useNavigate, useParams, NavLink } from "react-router-dom";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useDeleteRepository, useRepositories, useRepository } from "@/api/queries";
import { useAuth } from "@/auth/AuthProvider";
import { canManageUsers, canWriteRepository } from "@/auth/roles";
import { ArtifactsTable } from "@/components/repository/ArtifactsTable";
import { ConfirmDeleteDialog } from "@/components/repository/ConfirmDeleteDialog";
import { EcosystemBadge, ecosystemMeta } from "@/components/repository/EcosystemBadge";
import { EditAlloyMembersDialog } from "@/components/repository/EditAlloyMembersDialog";
import { KIND_META, RepositoryKindBadge } from "@/components/repository/RepositoryKindBadge";
import { QuotaPanel } from "@/components/repository/QuotaPanel";
import { RepositoryAccessPanel } from "@/components/repository/RepositoryAccessPanel";
import { AdmissionPanel } from "@/components/repository/AdmissionPanel";
import { RetentionPanel } from "@/components/repository/RetentionPanel";
import { WebhooksPanel } from "@/components/repository/WebhooksPanel";
import { SetMeUpDialog } from "@/components/repository/SetMeUpDialog";
import { UploadArtifactButton } from "@/components/repository/UploadArtifactButton";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
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
  const { data: repositories } = useRepositories();
  const deleteRepository = useDeleteRepository();
  const canWrite = canWriteRepository(repository?.access);
  const isAdmin = canManageUsers(user?.role);
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
    if (error instanceof ApiError && (error.status === 404 || error.status === 403)) {
      return <NotFoundPage message="Ese repositorio no existe o no tienes acceso." />;
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
  const isReadOnly =
    repository.kind.type === "mirror" || repository.kind.type === "alloy";
  const showAdmission =
    repository.kind.type === "forge" &&
    (repository.ecosystem === "oci" || repository.ecosystem === "helm");
  const alloyMembers =
    repository.kind.type === "alloy"
      ? repository.kind.members.map((memberId) => {
          const member = repositories?.find((item) => item.id === memberId);
          return { id: memberId, name: member?.name ?? memberId };
        })
      : [];

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
              {kind.label}
            </p>
            <h1 className="mt-1 truncate text-2xl font-semibold tracking-tight text-foreground">
              {repository.name}
            </h1>
            <div className="mt-3 flex flex-wrap items-center gap-2">
              <EcosystemBadge ecosystem={repository.ecosystem} />
              <RepositoryKindBadge kind={repository.kind} />
              {repository.restricted ? <Badge variant="outline">Restringido</Badge> : null}
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
            {alloyMembers.length > 0 ? (
              <p className="mt-3 text-sm text-muted-foreground">
                Agrega{" "}
                {alloyMembers.map((member, index) => (
                  <span key={member.id}>
                    {index > 0 ? ", " : null}
                    <NavLink
                      to={`/repositories/${member.id}`}
                      className="font-medium text-foreground underline-offset-4 hover:underline"
                    >
                      {member.name}
                    </NavLink>
                  </span>
                ))}
                . Las lecturas se resuelven en ese orden; publica en un Forge
                miembro.
              </p>
            ) : null}
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {repository.ecosystem === "cargo" ||
            repository.ecosystem === "npm" ||
            repository.ecosystem === "pypi" ||
            repository.ecosystem === "oci" ||
            repository.ecosystem === "helm" ||
            repository.ecosystem === "conan" ? (
              <SetMeUpDialog
                repositoryId={repositoryId}
                kind={repository.kind.type}
                ecosystem={repository.ecosystem}
              />
            ) : null}
            {canWrite && repository.kind.type === "alloy" ? (
              <EditAlloyMembersDialog
                repositoryId={repositoryId}
                ecosystem={repository.ecosystem}
                members={repository.kind.members}
              />
            ) : null}
            {canWrite && !isReadOnly && repository.ecosystem !== "oci" && repository.ecosystem !== "helm" && repository.ecosystem !== "conan" ? (
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

      <Tabs defaultValue="packages">
        <TabsList variant="line">
          <TabsTrigger value="packages">Paquetes</TabsTrigger>
          {showAdmission ? <TabsTrigger value="admission">Políticas</TabsTrigger> : null}
          {repository.kind.type !== "alloy" ? (
            <TabsTrigger value="retention">Retención</TabsTrigger>
          ) : null}
          {repository.kind.type !== "alloy" ? (
            <TabsTrigger value="quota">Cuota</TabsTrigger>
          ) : null}
          {repository.kind.type !== "alloy" ? (
            <TabsTrigger value="webhooks">Avisos</TabsTrigger>
          ) : null}
          {isAdmin ? <TabsTrigger value="access">Acceso</TabsTrigger> : null}
        </TabsList>
        <TabsContent value="packages" className="mt-4 space-y-3">
          <p className="text-xs text-muted-foreground">
            Agrupados por nombre, con cada versión debajo.
          </p>
          <ArtifactsTable
            repositoryId={repositoryId}
            kind={repository.kind.type}
            ecosystem={repository.ecosystem}
            canWrite={canWrite}
            memberNames={Object.fromEntries(
              alloyMembers.map((member) => [member.id, member.name]),
            )}
          />
        </TabsContent>
        {showAdmission ? (
          <TabsContent value="admission" className="mt-4">
            <AdmissionPanel repositoryId={repositoryId} canWrite={canWrite} />
          </TabsContent>
        ) : null}
        {repository.kind.type !== "alloy" ? (
          <TabsContent value="retention" className="mt-4">
            <RetentionPanel repositoryId={repositoryId} canWrite={canWrite} />
          </TabsContent>
        ) : null}
        {repository.kind.type !== "alloy" ? (
          <TabsContent value="quota" className="mt-4">
            <QuotaPanel repositoryId={repositoryId} canWrite={canWrite} />
          </TabsContent>
        ) : null}
        {repository.kind.type !== "alloy" ? (
          <TabsContent value="webhooks" className="mt-4">
            <WebhooksPanel repositoryId={repositoryId} canWrite={canWrite} />
          </TabsContent>
        ) : null}
        {isAdmin ? (
          <TabsContent value="access" className="mt-4">
            <RepositoryAccessPanel repositoryId={repositoryId} />
          </TabsContent>
        ) : null}
      </Tabs>
    </div>
  );
}
