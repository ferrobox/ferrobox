import { AlertCircle, Boxes, RefreshCw } from "lucide-react";
import { Link } from "react-router-dom";

import { useRepositories } from "@/api/queries";
import { useAuth } from "@/auth/AuthProvider";
import { canWriteArtifacts } from "@/auth/roles";
import { CreateRepositoryDialog } from "@/components/repository/CreateRepositoryDialog";
import { EcosystemBadge } from "@/components/repository/EcosystemBadge";
import { RepositoryKindBadge } from "@/components/repository/RepositoryKindBadge";
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

export function RepositoriesPage() {
  const { user } = useAuth();
  const { data, isPending, isError, error, refetch, isFetching } = useRepositories();
  const canWrite = canWriteArtifacts(user?.role);

  return (
    <div>
      <PageHeader
        title="Repositorios"
        description="Gestiona los repositorios de artefactos de tu organización."
        actions={canWrite ? <CreateRepositoryDialog /> : undefined}
      />

      {isPending ? <RepositoriesTableSkeleton /> : null}

      {isError ? (
        <Alert variant="destructive">
          <AlertCircle />
          <AlertTitle>No se pudieron cargar los repositorios</AlertTitle>
          <AlertDescription className="flex items-center justify-between gap-4">
            <span>{error.message}</span>
            <Button size="sm" variant="outline" onClick={() => void refetch()}>
              <RefreshCw className={isFetching ? "animate-spin" : ""} />
              Reintentar
            </Button>
          </AlertDescription>
        </Alert>
      ) : null}

      {data && data.length === 0 ? (
        <EmptyState />
      ) : data && data.length > 0 ? (
        <div className="overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Nombre</TableHead>
                <TableHead>Ecosistema</TableHead>
                <TableHead>Tipo</TableHead>
                <TableHead className="text-right">Identificador</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.map((repository) => (
                <TableRow key={repository.id} className="cursor-pointer">
                  <TableCell className="p-0">
                    <Link
                      to={`/repositories/${repository.id}`}
                      className="block px-4 py-3 font-medium text-foreground hover:underline"
                    >
                      {repository.name}
                    </Link>
                  </TableCell>
                  <TableCell>
                    <EcosystemBadge ecosystem={repository.ecosystem} />
                  </TableCell>
                  <TableCell>
                    <RepositoryKindBadge kind={repository.kind} />
                  </TableCell>
                  <TableCell className="text-right font-mono text-xs text-muted-foreground">
                    {repository.id}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      ) : null}
    </div>
  );
}

function RepositoriesTableSkeleton() {
  return (
    <div className="space-y-2">
      {Array.from({ length: 5 }, (_, index) => (
        <Skeleton key={index} className="h-12 w-full rounded-lg" />
      ))}
    </div>
  );
}

function EmptyState() {
  return (
    <div className="flex flex-col items-center justify-center gap-3 rounded-lg border border-dashed border-border py-20 text-center">
      <div className="flex size-12 items-center justify-center rounded-full bg-muted">
        <Boxes className="size-6 text-muted-foreground" />
      </div>
      <div className="space-y-1">
        <p className="font-medium text-foreground">Todavía no hay repositorios</p>
        <p className="text-sm text-muted-foreground">
          Crea tu primer repositorio para empezar a publicar artefactos.
        </p>
      </div>
      <CreateRepositoryDialog />
    </div>
  );
}
