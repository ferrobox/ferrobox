import { AlertCircle, Pencil, RefreshCw, Trash2 } from "lucide-react";
import { Link, Navigate, useNavigate } from "react-router-dom";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useDeleteGroup, useGroups } from "@/api/queries";
import { useAuth } from "@/auth/AuthProvider";
import { canManageUsers } from "@/auth/roles";
import { CreateGroupDialog } from "@/components/groups/CreateGroupDialog";
import { NameChips } from "@/components/groups/NameChips";
import { PageHeader } from "@/components/layout/PageHeader";
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

export function GroupsPage() {
  const { user } = useAuth();
  const navigate = useNavigate();
  const { data, isPending, isError, error, refetch, isFetching } = useGroups();
  const deleteGroup = useDeleteGroup();

  if (!canManageUsers(user?.role)) {
    return <Navigate to="/repositories" replace />;
  }

  async function onDelete(groupId: string, name: string) {
    try {
      await deleteGroup.mutateAsync(groupId);
      toast.success(`Grupo «${name}» eliminado`);
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo eliminar el grupo");
      throw err;
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title="Grupos"
        description="Pulsa un grupo para ver sus miembros y repositorios. Editar cambia quién pertenece y a qué tiene acceso. Un miembro de grupo solo ve esos repositorios. Quien no esté en ningún grupo sigue el rol de la instancia en los repositorios sin restringir."
        actions={<CreateGroupDialog />}
      />

      {isPending ? (
        <div className="space-y-2">
          {Array.from({ length: 3 }, (_, index) => (
            <Skeleton key={index} className="h-12 w-full rounded-lg" />
          ))}
        </div>
      ) : null}

      {isError ? (
        <Alert variant="destructive">
          <AlertCircle />
          <AlertTitle>No se pudieron cargar los grupos</AlertTitle>
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
        <p className="text-sm text-muted-foreground">
          Todavía no hay grupos. Crea uno para restringir el acceso a repositorios concretos.
        </p>
      ) : null}

      {data && data.length > 0 ? (
        <div className="overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Grupo</TableHead>
                <TableHead>Miembros</TableHead>
                <TableHead>Repositorios</TableHead>
                <TableHead className="text-right">Acciones</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.map((group) => (
                <TableRow
                  key={group.id}
                  className="cursor-pointer"
                  onClick={() => navigate(`/groups/${group.id}`)}
                >
                  <TableCell className="font-medium">
                    <Link
                      to={`/groups/${group.id}`}
                      className="hover:underline"
                      onClick={(event) => event.stopPropagation()}
                    >
                      {group.name}
                    </Link>
                  </TableCell>
                  <TableCell className="whitespace-normal">
                    <NameChips names={group.member_names} empty="Ninguno" />
                  </TableCell>
                  <TableCell className="whitespace-normal">
                    <NameChips names={group.repository_names} empty="Ninguno" />
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex justify-end gap-1">
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={(event) => {
                          event.stopPropagation();
                          navigate(`/groups/${group.id}?edit=1`);
                        }}
                      >
                        <Pencil />
                        Editar
                      </Button>
                      <ConfirmDeleteDialog
                        title={`Eliminar «${group.name}»`}
                        description="Se quitarán los miembros y el acceso a repositorios de este grupo. Los repositorios no se borran."
                        pending={deleteGroup.isPending}
                        onConfirm={() => onDelete(group.id, group.name)}
                        trigger={
                          <Button
                            size="sm"
                            variant="ghost"
                            onClick={(event) => event.stopPropagation()}
                          >
                            <Trash2 />
                            Eliminar
                          </Button>
                        }
                      />
                    </div>
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
