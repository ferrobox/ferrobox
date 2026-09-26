import { type FormEvent, useState } from "react";
import { Bot, Copy, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import type { RoleDto } from "@/api/generated/RoleDto";
import { useCreateRobot } from "@/api/queries";
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

const ROLE_OPTIONS: readonly RoleDto[] = ["developer", "reader"];

export function CreateRobotDialog() {
  const { t } = useTranslation();
  const createRobot = useCreateRobot();
  const [open, setOpen] = useState(false);
  const [username, setUsername] = useState("");
  const [tokenName, setTokenName] = useState("ci");
  const [expiresAt, setExpiresAt] = useState("");
  const [role, setRole] = useState<RoleDto>("developer");
  const [createdSecret, setCreatedSecret] = useState<string | null>(null);

  function resetForm() {
    setUsername("");
    setTokenName("ci");
    setExpiresAt("");
    setRole("developer");
  }

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    try {
      const result = await createRobot.mutateAsync({
        username: username.trim(),
        role,
        token_name: tokenName.trim(),
        expires_at: expiresAt.trim() ? new Date(expiresAt).toISOString() : undefined,
      });
      resetForm();
      setCreatedSecret(result.token.token);
      toast.success(t("users.robotCreated"));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("users.robotFailed"));
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) {
          resetForm();
          setCreatedSecret(null);
        }
      }}
    >
      <DialogTrigger asChild>
        <Button type="button" variant="outline">
          <Bot />
          {t("users.newRobot")}
        </Button>
      </DialogTrigger>
      <DialogContent>
        {createdSecret ? (
          <div className="grid gap-4">
            <DialogHeader>
              <DialogTitle>{t("security.copyNow")}</DialogTitle>
              <DialogDescription>{t("users.robotTokenHint")}</DialogDescription>
            </DialogHeader>
            <div className="flex items-center gap-2">
              <Input readOnly value={createdSecret} className="font-mono text-xs" />
              <Button
                type="button"
                variant="outline"
                size="icon"
                aria-label={t("security.copyToken")}
                onClick={() => {
                  void navigator.clipboard.writeText(createdSecret);
                  toast.success(t("security.tokenCopied"));
                }}
              >
                <Copy />
              </Button>
            </div>
            <DialogFooter>
              <Button type="button" onClick={() => setOpen(false)}>
                {t("security.saved")}
              </Button>
            </DialogFooter>
          </div>
        ) : (
          <form onSubmit={(event) => void onSubmit(event)} className="grid gap-4">
            <DialogHeader>
              <DialogTitle>{t("users.newRobot")}</DialogTitle>
              <DialogDescription>{t("users.newRobotHint")}</DialogDescription>
            </DialogHeader>
            <div className="space-y-2">
              <Label htmlFor="robot-username">{t("users.username")}</Label>
              <Input
                id="robot-username"
                value={username}
                onChange={(event) => setUsername(event.target.value)}
                required
                autoComplete="off"
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="robot-token-name">{t("security.tokenName")}</Label>
              <Input
                id="robot-token-name"
                value={tokenName}
                onChange={(event) => setTokenName(event.target.value)}
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="robot-expires">{t("security.expiresAt")}</Label>
              <Input
                id="robot-expires"
                type="datetime-local"
                value={expiresAt}
                onChange={(event) => setExpiresAt(event.target.value)}
              />
              <p className="text-xs text-muted-foreground">{t("security.expiresHint")}</p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="robot-role">{t("users.role")}</Label>
              <Select value={role} onValueChange={(value) => setRole(value as RoleDto)}>
                <SelectTrigger id="robot-role" className="w-full">
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
              <Button type="submit" disabled={createRobot.isPending}>
                {createRobot.isPending ? <Loader2 className="animate-spin" /> : <Bot />}
                {createRobot.isPending ? t("users.creating") : t("users.createRobot")}
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}
