import { AlertCircle, KeyRound, RefreshCw, Trash2 } from "lucide-react";
import { Navigate } from "react-router-dom";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useDeleteUser, useUpdateUserRole, useUsers } from "@/api/queries";
import type { RoleDto } from "@/api/generated/RoleDto";
import { useAuth } from "@/auth/AuthProvider";
import { canManageUsers, roleLabel } from "@/auth/roles";
import { PageHeader } from "@/components/layout/PageHeader";
import { ConfirmDeleteDialog } from "@/components/repository/ConfirmDeleteDialog";
import { CreateUserDialog } from "@/components/users/CreateUserDialog";
import { ResetPasswordDialog } from "@/components/users/ResetPasswordDialog";
import { RolePermissionsCard } from "@/components/users/RolePermissionsCard";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

const ROLE_OPTIONS: readonly RoleDto[] = ["admin", "developer", "reader"];

export function UsersPage() {
  const { user, updateCurrentUser } = useAuth();
  const { data, isPending, isError, error, refetch, isFetching } = useUsers();
  const deleteUser = useDeleteUser();
  const updateUserRole = useUpdateUserRole();

  if (!canManageUsers(user?.role)) {
    return <Navigate to="/repositories" replace />;
  }

  async function onChangeRole(userId: string, name: string, nextRole: RoleDto) {
    try {
      const updated = await updateUserRole.mutateAsync({ userId, role: nextRole });
      if (updated.id === user?.id) {
        updateCurrentUser(updated);
      }
      toast.success(`Rol de «${name}» actualizado`);
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo cambiar el rol");
    }
  }

  async function onDelete(userId: string, name: string) {
    try {
      await deleteUser.mutateAsync(userId);
      toast.success(`Usuario «${name}» eliminado`);
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo eliminar el usuario");
      throw err;
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title="Usuarios"
        description="Crea cuentas, asigna roles y restablece contraseñas. Si alguien olvida la suya, un administrador la restablece desde aquí."
        actions={<CreateUserDialog />}
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
          <AlertTitle>No se pudieron cargar los usuarios</AlertTitle>
          <AlertDescription className="flex items-center justify-between gap-4">
            <span>{error.message}</span>
            <Button size="sm" variant="outline" onClick={() => void refetch()}>
              <RefreshCw className={isFetching ? "animate-spin" : ""} />
              Reintentar
            </Button>
          </AlertDescription>
        </Alert>
      ) : null}

      {data ? (
        <div className="overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Usuario</TableHead>
                <TableHead>Correo</TableHead>
                <TableHead>Rol</TableHead>
                <TableHead className="text-right">Acciones</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.map((entry) => {
                const isSelf = entry.id === user?.id;
                return (
                  <TableRow key={entry.id}>
                    <TableCell className="font-medium">{entry.username}</TableCell>
                    <TableCell className="text-muted-foreground">
                      {entry.email ?? "—"}
                    </TableCell>
                    <TableCell>
                      <Select
                        value={entry.role}
                        disabled={updateUserRole.isPending}
                        onValueChange={(value) => {
                          const nextRole = value as RoleDto;
                          if (nextRole === entry.role) {
                            return;
                          }
                          void onChangeRole(entry.id, entry.username, nextRole);
                        }}
                      >
                        <SelectTrigger
                          aria-label={`Rol de ${entry.username}`}
                          className="h-8 w-[140px]"
                        >
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {ROLE_OPTIONS.map((option) => (
                            <SelectItem key={option} value={option}>
                              {roleLabel(option)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </TableCell>
                    <TableCell className="text-right">
                      <div className="flex justify-end gap-1">
                        <ResetPasswordDialog
                          userId={entry.id}
                          username={entry.username}
                          trigger={
                            <Button variant="ghost" size="sm" disabled={isSelf}>
                              <KeyRound />
                              Restablecer
                            </Button>
                          }
                        />
                        <ConfirmDeleteDialog
                          title={`Eliminar «${entry.username}»`}
                          description="Se revocarán sus tokens de API. Esta acción no se puede deshacer."
                          pending={deleteUser.isPending}
                          onConfirm={() => onDelete(entry.id, entry.username)}
                          trigger={
                            <Button variant="ghost" size="sm" disabled={isSelf}>
                              <Trash2 />
                              Eliminar
                            </Button>
                          }
                        />
                      </div>
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </div>
      ) : null}

      <RolePermissionsCard />
    </div>
  );
}
