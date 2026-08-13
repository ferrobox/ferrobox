import { AlertCircle, Download, FileBox, RefreshCw, Trash2 } from "lucide-react";
import { toast } from "sonner";

import { ApiError, downloadArtifact } from "@/api/client";
import { useDeleteArtifact, useRepositoryArtifacts } from "@/api/queries";
import { useAuth } from "@/auth/AuthProvider";
import { canWriteArtifacts } from "@/auth/roles";
import { ConfirmDeleteDialog } from "@/components/repository/ConfirmDeleteDialog";
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
import { formatBytes, truncateMiddle } from "@/lib/format";

export function ArtifactsTable({ repositoryId }: { repositoryId: string }) {
  const { user } = useAuth();
  const { data, isPending, isError, error, refetch, isFetching } =
    useRepositoryArtifacts(repositoryId);
  const deleteArtifact = useDeleteArtifact(repositoryId);
  const canWrite = canWriteArtifacts(user?.role);

  async function onDelete(artifactId: string) {
    try {
      await deleteArtifact.mutateAsync(artifactId);
      toast.success("Artefacto eliminado");
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo eliminar el artefacto");
      throw err;
    }
  }

  if (isPending) {
    return (
      <div className="space-y-2">
        {Array.from({ length: 3 }, (_, index) => (
          <Skeleton key={index} className="h-12 w-full rounded-lg" />
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

  if (data.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center gap-2 rounded-lg border border-dashed border-border py-16 text-center">
        <div className="flex size-12 items-center justify-center rounded-full bg-muted">
          <FileBox className="size-6 text-muted-foreground" />
        </div>
        <p className="font-medium text-foreground">Todavía no hay artefactos</p>
        <p className="text-sm text-muted-foreground">
          Sube el primero con el botón «Subir artefacto».
        </p>
      </div>
    );
  }

  return (
    <div className="overflow-hidden rounded-lg border border-border">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Identificador</TableHead>
            <TableHead>Checksum (SHA-256)</TableHead>
            <TableHead>Tamaño</TableHead>
            <TableHead className="text-right">Acciones</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {data.map((artifact) => (
            <TableRow key={artifact.id}>
              <TableCell className="font-mono text-xs">{truncateMiddle(artifact.id)}</TableCell>
              <TableCell className="font-mono text-xs text-muted-foreground">
                {truncateMiddle(artifact.checksum)}
              </TableCell>
              <TableCell className="text-sm text-muted-foreground">
                {formatBytes(artifact.size_bytes)}
              </TableCell>
              <TableCell className="text-right">
                <div className="flex justify-end gap-1">
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      void downloadArtifact(artifact.id).catch((err: unknown) => {
                        toast.error(
                          err instanceof ApiError
                            ? err.message
                            : "No se pudo descargar el artefacto",
                        );
                      });
                    }}
                  >
                    <Download />
                    Descargar
                  </Button>
                  {canWrite ? (
                    <ConfirmDeleteDialog
                      title="Eliminar artefacto"
                      description="Se borrarán el objeto almacenado, los metadatos y la entrada de índice asociada. Esta acción no se puede deshacer."
                      pending={deleteArtifact.isPending}
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
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  );
}
