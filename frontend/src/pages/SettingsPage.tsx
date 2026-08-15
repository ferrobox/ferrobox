import { type FormEvent, useState } from "react";
import { Check, Copy, KeyRound } from "lucide-react";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useChangePassword, useSettings } from "@/api/queries";
import { useAuth } from "@/auth/AuthProvider";
import { roleLabel } from "@/auth/roles";
import { PageHeader } from "@/components/layout/PageHeader";
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
        description="Cuenta, contraseña y datos de esta instancia de FerroBox."
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
            </form>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Instancia</CardTitle>
            <CardDescription>
              URL pública que deben usar Cargo y otros clientes (
              <code className="font-mono">PUBLIC_BASE_URL</code>) y retención de
              Mirrors npm / PyPI (
              <code className="font-mono">MIRROR_QUARANTINE_DAYS</code>).
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
                <div>
                  <p className="text-sm text-muted-foreground">
                    Cuarentena de Mirrors npm / PyPI
                  </p>
                  <p className="mt-1 font-mono text-sm text-foreground">
                    {settings?.mirror_quarantine_days === 0
                      ? "Desactivada"
                      : `${settings?.mirror_quarantine_days ?? "—"} días`}
                  </p>
                  <p className="mt-2 text-xs text-muted-foreground">
                    {settings?.mirror_quarantine_days === 0
                      ? "Las versiones nuevas de npm y PyPI públicos se sirven al momento. El valor por defecto es 14."
                      : "Las versiones publicadas en npm o PyPI hace menos de ese plazo no aparecen en el índice y la descarga responde 403. Reduce la ventana de typosquatting. No aplica a Forges."}{" "}
                    Se toma de <code className="font-mono">MIRROR_QUARANTINE_DAYS</code>
                    {settings?.mirror_quarantine_days === 0 ? "." : " (0 la desactiva)."}
                  </p>
                </div>
              </>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
