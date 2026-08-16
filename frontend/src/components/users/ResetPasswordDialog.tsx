import { type FormEvent, type ReactNode, useState } from "react";
import { KeyRound, Loader2 } from "lucide-react";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useResetUserPassword } from "@/api/queries";
import { PASSWORD_POLICY_HINT, passwordMeetsPolicy } from "@/auth/passwordPolicy";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export function ResetPasswordDialog({
  userId,
  username,
  trigger,
}: {
  userId: string;
  username: string;
  trigger: ReactNode;
}) {
  const resetPassword = useResetUserPassword();
  const [open, setOpen] = useState(false);
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");

  function resetForm() {
    setPassword("");
    setConfirmPassword("");
  }

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();

    if (!passwordMeetsPolicy(password)) {
      toast.error(PASSWORD_POLICY_HINT);
      return;
    }

    if (password !== confirmPassword) {
      toast.error("La contraseña y su confirmación no coinciden");
      return;
    }

    try {
      await resetPassword.mutateAsync({ userId, password });
      resetForm();
      setOpen(false);
      toast.success(`Contraseña de «${username}» restablecida`);
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "No se pudo restablecer la contraseña",
      );
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) {
          resetForm();
        }
      }}
    >
      <DialogTrigger asChild>{trigger}</DialogTrigger>
      <DialogContent>
        <form onSubmit={(event) => void onSubmit(event)} className="grid gap-4">
          <DialogHeader>
            <DialogTitle>Restablecer contraseña</DialogTitle>
            <DialogDescription>
              Asigna una contraseña nueva a «{username}». No puedes restablecer la tuya aquí:
              usa Configuración.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-2">
            <Label htmlFor={`reset-password-${userId}`}>Nueva contraseña</Label>
            <Input
              id={`reset-password-${userId}`}
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              required
              autoComplete="new-password"
            />
            <p className="text-xs text-muted-foreground">{PASSWORD_POLICY_HINT}</p>
          </div>
          <div className="space-y-2">
            <Label htmlFor={`reset-password-confirm-${userId}`}>Confirmar contraseña</Label>
            <Input
              id={`reset-password-confirm-${userId}`}
              type="password"
              value={confirmPassword}
              onChange={(event) => setConfirmPassword(event.target.value)}
              required
              autoComplete="new-password"
            />
          </div>
          <DialogFooter>
            <Button type="submit" disabled={resetPassword.isPending}>
              {resetPassword.isPending ? <Loader2 className="animate-spin" /> : <KeyRound />}
              {resetPassword.isPending ? "Guardando…" : "Restablecer"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
