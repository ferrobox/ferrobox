import { Check, Minus } from "lucide-react";

import { roleLabel } from "@/auth/roles";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

const ROLES = ["reader", "developer", "admin"] as const;

type RoleKey = (typeof ROLES)[number];

interface PermissionRow {
  readonly label: string;
  readonly allowed: Readonly<Record<RoleKey, boolean>>;
}

const PERMISSIONS: readonly PermissionRow[] = [
  {
    label: "Ver repositorios, paquetes y ensayes",
    allowed: { reader: true, developer: true, admin: true },
  },
  {
    label: "Descargar e instalar paquetes",
    allowed: { reader: true, developer: true, admin: true },
  },
  {
    label: "Buscar en el catálogo",
    allowed: { reader: true, developer: true, admin: true },
  },
  {
    label: "Gestionar tokens de API propios y cambiar la propia contraseña",
    allowed: { reader: true, developer: true, admin: true },
  },
  {
    label: "Publicar y borrar artefactos",
    allowed: { reader: false, developer: true, admin: true },
  },
  {
    label: "Crear y borrar repositorios",
    allowed: { reader: false, developer: true, admin: true },
  },
  {
    label: "Configurar retención y cuota",
    allowed: { reader: false, developer: true, admin: true },
  },
  {
    label: "Recolección de basura y relanzar ensayes",
    allowed: { reader: false, developer: true, admin: true },
  },
  {
    label: "Crear usuarios, cambiar roles y borrar cuentas",
    allowed: { reader: false, developer: false, admin: true },
  },
  {
    label: "Crear grupos y asignar repositorios",
    allowed: { reader: false, developer: false, admin: true },
  },
  {
    label: "Restablecer la contraseña de otra cuenta",
    allowed: { reader: false, developer: false, admin: true },
  },
];

export function RolePermissionsCard() {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Permisos por rol</CardTitle>
        <CardDescription>
          Los roles aplican a toda la instancia. Un Lector lee; un Desarrollador además publica y
          administra repositorios; un Administrador también gestiona cuentas y grupos. Si un
          repositorio tiene grupos asignados, solo esos grupos (y los administradores) pueden
          verlo; el rol del grupo en ese repositorio puede conceder escritura a un Lector.
        </CardDescription>
      </CardHeader>
      <CardContent className="px-0 sm:px-6">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Acción</TableHead>
              {ROLES.map((role) => (
                <TableHead key={role} className="text-center">
                  {roleLabel(role)}
                </TableHead>
              ))}
            </TableRow>
          </TableHeader>
          <TableBody>
            {PERMISSIONS.map((row) => (
              <TableRow key={row.label}>
                <TableCell className="whitespace-normal font-medium">{row.label}</TableCell>
                {ROLES.map((role) => (
                  <TableCell key={role} className="text-center">
                    {row.allowed[role] ? (
                      <Check
                        className="mx-auto size-4 text-emerald-600"
                        aria-label="Permitido"
                      />
                    ) : (
                      <Minus className="mx-auto size-4 text-muted-foreground" aria-label="No" />
                    )}
                  </TableCell>
                ))}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </CardContent>
    </Card>
  );
}
