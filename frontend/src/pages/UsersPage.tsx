import { type FormEvent, useState } from "react";
import { AlertCircle, RefreshCw, Trash2, UserPlus } from "lucide-react";
import { Navigate } from "react-router-dom";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useCreateUser, useDeleteUser, useUsers } from "@/api/queries";
import type { RoleDto } from "@/api/generated/RoleDto";
import { useAuth } from "@/auth/AuthProvider";
import { canManageUsers, roleLabel } from "@/auth/roles";
import { PageHeader } from "@/components/layout/PageHeader";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
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
  const { user } = useAuth();
  const { data, isPending, isError, error, refetch, isFetching } = useUsers();
  const createUser = useCreateUser();
  const deleteUser = useDeleteUser();

  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [role, setRole] = useState<RoleDto>("developer");

  if (!canManageUsers(user?.role)) {
    return <Navigate to="/repositories" replace />;
  }

  async function onCreate(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    try {
      await createUser.mutateAsync({
        username: username.trim(),
        password,
        role,
      });
      setUsername("");
      setPassword("");
      setRole("developer");
      toast.success("Usuario creado");
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo crear el usuario");
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
        description="Crea cuentas y asigna roles (admin, developer, reader)."
      />

      <form
        onSubmit={(event) => void onCreate(event)}
        className="mb-8 grid gap-3 sm:grid-cols-2 lg:grid-cols-4 lg:items-end"
      >
        <div className="space-y-2">
          <Label htmlFor="new-username">Usuario</Label>
          <Input
            id="new-username"
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
            type="password"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            required
            autoComplete="new-password"
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="new-role">Rol</Label>
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
        <Button
          type="submit"
          disabled={createUser.isPending || username.trim().length === 0 || password.length === 0}
        >
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
                    <Badge variant="secondary">{roleLabel(entry.role)}</Badge>
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
