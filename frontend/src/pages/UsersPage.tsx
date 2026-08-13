import { type FormEvent, useState } from "react";
import { AlertCircle, RefreshCw, Trash2, UserPlus } from "lucide-react";
import { Navigate } from "react-router-dom";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useCreateUser, useDeleteUser, useUpdateUserRole, useUsers } from "@/api/queries";
import type { RoleDto } from "@/api/generated/RoleDto";
import { useAuth } from "@/auth/AuthProvider";
import { canManageUsers, roleLabel } from "@/auth/roles";
import { PageHeader } from "@/components/layout/PageHeader";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
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
  const createUser = useCreateUser();
  const deleteUser = useDeleteUser();
  const updateUserRole = useUpdateUserRole();

  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [role, setRole] = useState<RoleDto>("developer");

  if (!canManageUsers(user?.role)) {
    return <Navigate to="/repositories" replace />;
  }

  async function onCreate(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = event.currentTarget;
    const data = new FormData(form);
    const nextUsername = String(data.get("username") ?? "").trim();
    const nextPassword = String(data.get("password") ?? "");
    const nextRole = (String(data.get("role") ?? role) || "developer") as RoleDto;

    if (nextUsername.length === 0 || nextPassword.length === 0) {
      toast.error("Usuario y contraseña son obligatorios");
      return;
    }

    try {
      await createUser.mutateAsync({
        username: nextUsername,
        password: nextPassword,
        role: nextRole,
      });
      setUsername("");
      setPassword("");
      setRole("developer");
      form.reset();
      toast.success("Usuario creado");
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo crear el usuario");
    }
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
    }
  }

  return (
    <div>
      <PageHeader
        title="Usuarios"
        description="Crea cuentas y asigna o cambia roles (admin, developer, reader)."
      />

      <form
        onSubmit={(event) => void onCreate(event)}
        className="mb-8 grid gap-3 sm:grid-cols-2 lg:grid-cols-4 lg:items-end"
      >
        <div className="space-y-2">
          <Label htmlFor="new-username">Usuario</Label>
          <Input
            id="new-username"
            name="username"
            value={username}
            onChange={(event) => setUsername(event.target.value)}
            required
            autoComplete="off"
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="new-password">Contraseña</Label>
          <Input
            id="new-password"
            name="password"
            type="password"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            required
            autoComplete="new-password"
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="new-role">Rol</Label>
          <input type="hidden" name="role" value={role} />
          <Select value={role} onValueChange={(value) => setRole(value as RoleDto)}>
            <SelectTrigger id="new-role" className="w-full">
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
        </div>
        <Button type="submit" disabled={createUser.isPending}>
          <UserPlus />
          {createUser.isPending ? "Creando…" : "Crear usuario"}
        </Button>
      </form>

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
                <TableHead>Rol</TableHead>
                <TableHead className="text-right">Acciones</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.map((entry) => (
                <TableRow key={entry.id}>
                  <TableCell className="font-medium">{entry.username}</TableCell>
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
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={deleteUser.isPending || entry.id === user?.id}
                      onClick={() => void onDelete(entry.id, entry.username)}
                    >
                      <Trash2 />
                      Eliminar
                    </Button>
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
