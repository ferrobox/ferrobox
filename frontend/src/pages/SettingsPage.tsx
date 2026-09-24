import { type FormEvent, useState } from "react";
import { AlertCircle, Check, Copy, KeyRound, RefreshCw } from "lucide-react";
import { Link } from "react-router-dom";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useChangePassword, useMyGroups, useSettings } from "@/api/queries";
import { useAuth } from "@/auth/AuthProvider";
import { PASSWORD_POLICY_HINT, passwordMeetsPolicy } from "@/auth/passwordPolicy";
import { canManageUsers, roleLabel } from "@/auth/roles";
import { GarbageCollectionCard } from "@/components/cleanup/GarbageCollectionCard";
import { PageHeader } from "@/components/layout/PageHeader";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";

export function SettingsPage() {
  const { user } = useAuth();
  const { data: settings, isPending } = useSettings();
  const changePassword = useChangePassword();

  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [copiedUrl, setCopiedUrl] = useState(false);

  async function onChangePassword(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();

    if (newPassword !== confirmPassword) {
      toast.error("La nueva contraseña y su confirmación no coinciden");
      return;
    }

    if (!passwordMeetsPolicy(newPassword)) {
      toast.error(PASSWORD_POLICY_HINT);
      return;
    }

    try {
      await changePassword.mutateAsync({
        current_password: currentPassword,
        new_password: newPassword,
      });
      setCurrentPassword("");
      setNewPassword("");
      setConfirmPassword("");
      toast.success("Contraseña actualizada");
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo cambiar la contraseña");
    }
  }

  async function copyPublicUrl() {
    if (!settings) {
      return;
    }
    await navigator.clipboard.writeText(settings.public_base_url);
    setCopiedUrl(true);
    window.setTimeout(() => setCopiedUrl(false), 1500);
    toast.success("URL copiada");
  }

  const canSubmit =
    currentPassword.length > 0 &&
    newPassword.length > 0 &&
    confirmPassword.length > 0 &&
    !changePassword.isPending;

  return (
    <div>
      <PageHeader
        title="Configuración"
        description="Cuenta, instancia y recolección de basura de FerroBox."
      />

      <div className="grid gap-6 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle>Cuenta</CardTitle>
            <CardDescription>Identidad de la sesión actual y cambio de contraseña.</CardDescription>
          </CardHeader>
          <CardContent className="space-y-6">
            <dl className="grid gap-3 text-sm sm:grid-cols-2">
              <div>
                <dt className="text-muted-foreground">Usuario</dt>
                <dd className="mt-1 font-medium text-foreground">{user?.username ?? "—"}</dd>
              </div>
              <div>
                <dt className="text-muted-foreground">Correo</dt>
                <dd className="mt-1 font-medium text-foreground">{user?.email ?? "—"}</dd>
              </div>
              <div>
                <dt className="text-muted-foreground">Rol</dt>
                <dd className="mt-1">
                  {user ? <Badge variant="outline">{roleLabel(user.role)}</Badge> : "—"}
                </dd>
              </div>
            </dl>

            <form onSubmit={(event) => void onChangePassword(event)} className="space-y-4">
              <div className="space-y-2">
                <Label htmlFor="current-password">Contraseña actual</Label>
                <Input
                  id="current-password"
                  type="password"
                  autoComplete="current-password"
                  value={currentPassword}
                  onChange={(event) => setCurrentPassword(event.target.value)}
                  required
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="new-password">Nueva contraseña</Label>
                <Input
                  id="new-password"
                  type="password"
                  autoComplete="new-password"
                  value={newPassword}
                  onChange={(event) => setNewPassword(event.target.value)}
                  required
                />
                <p className="text-xs text-muted-foreground">{PASSWORD_POLICY_HINT}</p>
              </div>
              <div className="space-y-2">
                <Label htmlFor="confirm-password">Confirmar nueva contraseña</Label>
                <Input
                  id="confirm-password"
                  type="password"
                  autoComplete="new-password"
                  value={confirmPassword}
                  onChange={(event) => setConfirmPassword(event.target.value)}
                  required
                />
              </div>
              <Button type="submit" disabled={!canSubmit}>
                <KeyRound />
                {changePassword.isPending ? "Guardando…" : "Cambiar contraseña"}
              </Button>
              <p className="text-xs text-muted-foreground">
                Si olvidaste la contraseña, pide a un administrador que la restablezca. No se
                envían correos de recuperación.
              </p>
            </form>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Instancia</CardTitle>
            <CardDescription>
              URL pública que deben usar Cargo y otros clientes. Se toma de{" "}
              <code className="font-mono">PUBLIC_BASE_URL</code>.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-5">
            {isPending ? (
              <div className="space-y-2">
                <Skeleton className="h-10 w-full rounded-md" />
                <Skeleton className="h-6 w-32 rounded-md" />
              </div>
            ) : (
              <>
                <div className="space-y-2">
                  <Label htmlFor="public-base-url">URL pública</Label>
                  <div className="flex items-center gap-2">
                    <Input
                      id="public-base-url"
                      readOnly
                      value={settings?.public_base_url ?? ""}
                      className="font-mono text-xs"
                    />
                    <Button
                      type="button"
                      variant="outline"
                      size="icon"
                      aria-label="Copiar URL pública"
                      onClick={() => void copyPublicUrl()}
                    >
                      {copiedUrl ? <Check /> : <Copy />}
                    </Button>
                  </div>
                </div>
                <div>
                  <p className="text-sm text-muted-foreground">Versión del servidor</p>
                  <p className="mt-1 font-mono text-sm text-foreground">
                    {settings?.version ?? "—"}
                  </p>
                </div>
              </>
            )}
          </CardContent>
        </Card>
        <div className="lg:col-span-2">
          <MembershipsCard />
        </div>
        <GarbageCollectionCard />
      </div>
    </div>
  );
}

function MembershipsCard() {
  const { user } = useAuth();
  const { data, isPending, isError, error, refetch, isFetching } = useMyGroups();
  const admin = canManageUsers(user?.role);

  return (
    <Card>
      <CardHeader>
        <CardTitle>Grupos</CardTitle>
        <CardDescription>
          Los grupos a los que perteneces determinan qué repositorios ves.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {isPending ? (
          <div className="space-y-2">
            <Skeleton className="h-10 w-full rounded-md" />
            <Skeleton className="h-10 w-2/3 rounded-md" />
          </div>
        ) : null}

        {isError ? (
          <Alert variant="destructive">
            <AlertCircle />
            <AlertTitle>No se pudieron cargar tus grupos</AlertTitle>
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
            {admin
              ? "No perteneces a ningún grupo. Como Admin ves todos los repositorios."
              : "No perteneces a ningún grupo. Ves los repositorios que no están restringidos, según tu rol de instancia."}
          </p>
        ) : null}

        {data && data.length > 0 ? (
          <ul className="space-y-3">
            {data.map((group) => (
              <li key={group.id} className="rounded-md border border-border px-3 py-2">
                <p className="font-medium text-foreground">{group.name}</p>
                {group.repositories.length === 0 ? (
                  <p className="mt-1 text-sm text-muted-foreground">
                    Este grupo aún no tiene repositorios asignados.
                  </p>
                ) : (
                  <ul className="mt-2 space-y-1">
                    {group.repositories.map((grant) => (
                      <li
                        key={grant.repository_id}
                        className="flex items-center justify-between gap-2 text-sm"
                      >
                        <span className="min-w-0 truncate">{grant.repository_name}</span>
                        <Badge variant="outline">{roleLabel(grant.role)}</Badge>
                      </li>
                    ))}
                  </ul>
                )}
              </li>
            ))}
          </ul>
        ) : null}

        {admin ? (
          <Button variant="outline" size="sm" asChild>
            <Link to="/groups">Gestionar grupos</Link>
          </Button>
        ) : null}
      </CardContent>
    </Card>
  );
}
