import { Outlet, useParams } from "react-router-dom";

import { useRepositories } from "@/api/queries";
import { RepositoryBrowser } from "@/components/repository/RepositoryBrowser";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { AlertCircle, RefreshCw } from "lucide-react";

export function RepositoriesLayout() {
  const { repositoryId } = useParams<{ repositoryId: string }>();
  const { data, isPending, isError, error, refetch, isFetching } = useRepositories();

  return (
    <div className="flex min-h-0 flex-1">
      {isPending ? (
        <aside className="flex h-full w-80 shrink-0 flex-col border-r border-border p-4">
          <Skeleton className="mb-3 h-9 w-full" />
          <Skeleton className="mb-2 h-8 w-full" />
          <Skeleton className="h-8 w-3/4" />
        </aside>
      ) : isError ? (
        <aside className="flex h-full w-80 shrink-0 flex-col border-r border-border p-4">
          <Alert variant="destructive">
            <AlertCircle />
            <AlertTitle>No se pudieron cargar los repositorios</AlertTitle>
            <AlertDescription className="flex flex-col gap-2">
              <span>{error.message}</span>
              <Button size="sm" variant="outline" onClick={() => void refetch()}>
                <RefreshCw className={isFetching ? "animate-spin" : ""} />
                Reintentar
              </Button>
            </AlertDescription>
          </Alert>
        </aside>
      ) : (
        <RepositoryBrowser repositories={data ?? []} selectedId={repositoryId} />
      )}
      <div className="min-w-0 flex-1 overflow-y-auto px-8 py-8">
        <Outlet />
      </div>
    </div>
  );
}
