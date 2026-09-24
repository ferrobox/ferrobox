import { AlertCircle, RefreshCw } from "lucide-react";
import { Navigate } from "react-router-dom";

import { useAuditEvents } from "@/api/queries";
import { useAuth } from "@/auth/AuthProvider";
import { canManageUsers } from "@/auth/roles";
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

const ACTION_LABELS: Record<string, string> = {
  "user.created": "Creó un usuario",
  "user.deleted": "Eliminó un usuario",
  "user.role_changed": "Cambió el rol",
  "user.password_reset": "Restableció una contraseña",
  "user.password_changed": "Cambió su contraseña",
  "token.created": "Creó un token",
  "token.revoked": "Revocó un token",
  "group.created": "Creó un grupo",
  "group.deleted": "Eliminó un grupo",
  "group.members_changed": "Cambió los miembros de un grupo",
  "group.repositories_changed": "Cambió el acceso de un grupo",
  "repository.created": "Creó un repositorio",
  "repository.deleted": "Eliminó un repositorio",
  "repository.members_changed": "Cambió los miembros de un Alloy",
  "artifact.published": "Publicó un artefacto",
  "artifact.deleted": "Eliminó un artefacto",
  "package.published": "Publicó un paquete",
  "package.yanked": "Yankeó un paquete",
  "package.unyanked": "Deshizo un yank",
  "package.promoted": "Promovió un paquete",
  "admission.policy_changed": "Cambió la política de admisión",
  "retention.policy_changed": "Cambió la retención",
  "retention.applied": "Aplicó la retención",
  "retention.gc": "Ejecutó la recolección de basura",
  "quota.changed": "Cambió la cuota",
  "webhook.created": "Creó un aviso",
  "webhook.updated": "Actualizó un aviso",
  "webhook.deleted": "Eliminó un aviso",
};

function formatWhen(value: string): string {
  const parsed = Date.parse(value);
  if (Number.isNaN(parsed)) {
    return value;
  }
  return new Intl.DateTimeFormat("es", {
    dateStyle: "short",
    timeStyle: "medium",
  }).format(parsed);
}

export function AuditPage() {
  const { user } = useAuth();
  const { data, isPending, isError, error, refetch, isFetching } = useAuditEvents(
    canManageUsers(user?.role),
  );

  if (!canManageUsers(user?.role)) {
    return <Navigate to="/repositories" replace />;
  }

  return (
    <div>
      <PageHeader
        title="Auditoría"
        description="Quién publicó, yankeó, cambió un grupo o creó un usuario. Las últimas 200 escrituras de la instancia."
        actions={
          <Button size="sm" variant="outline" onClick={() => void refetch()}>
            <RefreshCw className={isFetching ? "animate-spin" : ""} />
            Actualizar
          </Button>
        }
      />

      {isPending ? (
        <div className="space-y-2">
          {Array.from({ length: 4 }, (_, index) => (
            <Skeleton key={index} className="h-12 w-full rounded-lg" />
          ))}
        </div>
      ) : null}

      {isError ? (
        <Alert variant="destructive">
          <AlertCircle />
          <AlertTitle>No se pudo cargar el registro</AlertTitle>
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
        <p className="text-sm text-muted-foreground">Aún no hay escrituras registradas.</p>
      ) : null}

      {data && data.length > 0 ? (
        <div className="overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Cuándo</TableHead>
                <TableHead>Quién</TableHead>
                <TableHead>Acción</TableHead>
                <TableHead>Destino</TableHead>
                <TableHead>Detalle</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.map((event) => (
                <TableRow key={event.id}>
                  <TableCell className="whitespace-nowrap text-muted-foreground">
                    {formatWhen(event.created_at)}
                  </TableCell>
                  <TableCell className="font-medium">{event.actor}</TableCell>
                  <TableCell>{ACTION_LABELS[event.action] ?? event.action}</TableCell>
                  <TableCell>
                    <span className="text-muted-foreground">{event.target_kind}</span>
                    {event.target ? (
                      <>
                        {" "}
                        <span className="font-medium">{event.target}</span>
                      </>
                    ) : null}
                  </TableCell>
                  <TableCell className="text-muted-foreground">{event.detail || "—"}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      ) : null}
    </div>
  );
}
