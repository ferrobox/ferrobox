import { type FormEvent, useState } from "react";
import { Loader2, UserPlus } from "lucide-react";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import type { RoleDto } from "@/api/generated/RoleDto";
import { useCreateUser } from "@/api/queries";
import { PASSWORD_POLICY_HINT, passwordMeetsPolicy } from "@/auth/passwordPolicy";
import { roleLabel } from "@/auth/roles";
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

const ROLE_OPTIONS: readonly RoleDto[] = ["admin", "developer", "reader"];

export function CreateUserDialog() {
  const createUser = useCreateUser();
  const [open, setOpen] = useState(false);
  const [username, setUsername] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [role, setRole] = useState<RoleDto>("developer");

  function resetForm() {
    setUsername("");
    setEmail("");
    setPassword("");
    setConfirmPassword("");
    setRole("developer");
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
      await createUser.mutateAsync({
        username: username.trim(),
        email: email.trim(),
        password,
        role,
      });
      resetForm();
      setOpen(false);
      toast.success("Usuario creado");
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo crear el usuario");
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
      <DialogTrigger asChild>
        <Button type="button">
          <UserPlus />
          Nuevo usuario
        </Button>
      </DialogTrigger>
      <DialogContent>
        <form onSubmit={(event) => void onSubmit(event)} className="grid gap-4">
          <DialogHeader>
            <DialogTitle>Nuevo usuario</DialogTitle>
            <DialogDescription>
              El nombre de usuario y el correo deben ser únicos. La contraseña debe cumplir la
              política de la instancia.
            </DialogDescription>
          </DialogHeader>
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
            <Label htmlFor="new-email">Correo</Label>
            <Input
              id="new-email"
              type="email"
              value={email}
              onChange={(event) => setEmail(event.target.value)}
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
            <p className="text-xs text-muted-foreground">{PASSWORD_POLICY_HINT}</p>
          </div>
          <div className="space-y-2">
            <Label htmlFor="new-password-confirm">Confirmar contraseña</Label>
            <Input
              id="new-password-confirm"
              type="password"
              value={confirmPassword}
              onChange={(event) => setConfirmPassword(event.target.value)}
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
          <DialogFooter>
            <Button type="submit" disabled={createUser.isPending}>
              {createUser.isPending ? <Loader2 className="animate-spin" /> : <UserPlus />}
              {createUser.isPending ? "Creando…" : "Crear usuario"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
