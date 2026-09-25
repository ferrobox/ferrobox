import { type FormEvent, type ReactNode, useState } from "react";
import { KeyRound, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useResetUserPassword } from "@/api/queries";
import { passwordMeetsPolicy } from "@/auth/passwordPolicy";
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
  const { t } = useTranslation();
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
      toast.error(t("password.policy"));
      return;
    }

    if (password !== confirmPassword) {
      toast.error(t("users.mismatch"));
      return;
    }

    try {
      await resetPassword.mutateAsync({ userId, password });
      resetForm();
      setOpen(false);
      toast.success(t("users.resetDone", { name: username }));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("users.resetFailed"));
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
            <DialogTitle>{t("users.resetTitle")}</DialogTitle>
            <DialogDescription>{t("users.resetHint", { name: username })}</DialogDescription>
          </DialogHeader>
          <div className="space-y-2">
            <Label htmlFor={`reset-password-${userId}`}>{t("settings.newPassword")}</Label>
            <Input
              id={`reset-password-${userId}`}
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              required
              autoComplete="new-password"
            />
            <p className="text-xs text-muted-foreground">{t("password.policy")}</p>
          </div>
          <div className="space-y-2">
            <Label htmlFor={`reset-password-confirm-${userId}`}>{t("users.confirmPassword")}</Label>
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
              {resetPassword.isPending ? t("common.saving") : t("users.reset")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
