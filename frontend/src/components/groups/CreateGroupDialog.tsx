import { type FormEvent, useState } from "react";
import { Loader2, UsersRound } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useCreateGroup } from "@/api/queries";
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

function isValidGroupName(name: string): boolean {
  return /^[A-Za-z0-9_-]{1,64}$/.test(name);
}

export function CreateGroupDialog() {
  const { t } = useTranslation();
  const createGroup = useCreateGroup();
  const navigate = useNavigate();
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = name.trim();
    if (!isValidGroupName(trimmed)) {
      toast.error(t("groups.nameHint"));
      return;
    }
    try {
      const created = await createGroup.mutateAsync({ name: trimmed });
      setName("");
      setOpen(false);
      toast.success(t("groups.created"));
      navigate(`/groups/${created.id}?edit=1`);
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("groups.createFailed"));
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) {
          setName("");
        }
      }}
    >
      <DialogTrigger asChild>
        <Button>
          <UsersRound />
          {t("groups.new")}
        </Button>
      </DialogTrigger>
      <DialogContent>
        <form onSubmit={(event) => void onSubmit(event)}>
          <DialogHeader>
            <DialogTitle>{t("groups.create")}</DialogTitle>
            <DialogDescription>{t("groups.newHint")}</DialogDescription>
          </DialogHeader>
          <div className="grid gap-4 py-4">
            <div className="grid gap-2">
              <Label htmlFor="group-name">{t("repositories.name")}</Label>
              <Input
                id="group-name"
                autoComplete="off"
                value={name}
                onChange={(event) => setName(event.target.value)}
                placeholder={t("groups.namePlaceholder")}
              />
              <p className="text-xs text-muted-foreground">{t("groups.nameHint")}</p>
            </div>
          </div>
          <DialogFooter>
            <Button type="submit" disabled={createGroup.isPending || name.trim().length === 0}>
              {createGroup.isPending ? <Loader2 className="animate-spin" /> : null}
              {t("common.create")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
