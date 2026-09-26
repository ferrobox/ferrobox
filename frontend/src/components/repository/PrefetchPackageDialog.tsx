import { type FormEvent, useState } from "react";
import { CloudDownload, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { usePrefetchPackage } from "@/api/queries";
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

export function PrefetchPackageDialog({ repositoryId }: { repositoryId: string }) {
  const { t } = useTranslation();
  const prefetch = usePrefetchPackage(repositoryId);
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [version, setVersion] = useState("");

  function resetForm() {
    setName("");
    setVersion("");
  }

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    try {
      const result = await prefetch.mutateAsync({
        name: name.trim(),
        version: version.trim() || undefined,
      });
      resetForm();
      setOpen(false);
      toast.success(
        t("repositories.prefetched", {
          name: result.version ? `${result.name}@${result.version}` : result.name,
        }),
      );
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("repositories.prefetchFailed"));
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
        <Button type="button" variant="outline">
          <CloudDownload />
          {t("repositories.prefetch")}
        </Button>
      </DialogTrigger>
      <DialogContent>
        <form onSubmit={(event) => void onSubmit(event)} className="grid gap-4">
          <DialogHeader>
            <DialogTitle>{t("repositories.prefetch")}</DialogTitle>
            <DialogDescription>{t("repositories.prefetchHint")}</DialogDescription>
          </DialogHeader>
          <div className="space-y-2">
            <Label htmlFor="prefetch-name">{t("repositories.prefetchName")}</Label>
            <Input
              id="prefetch-name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              required
              autoComplete="off"
              placeholder="libc"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="prefetch-version">{t("repositories.prefetchVersion")}</Label>
            <Input
              id="prefetch-version"
              value={version}
              onChange={(event) => setVersion(event.target.value)}
              autoComplete="off"
              placeholder="0.2.177"
            />
            <p className="text-xs text-muted-foreground">{t("repositories.prefetchVersionHint")}</p>
          </div>
          <DialogFooter>
            <Button type="submit" disabled={prefetch.isPending || name.trim().length === 0}>
              {prefetch.isPending ? <Loader2 className="animate-spin" /> : <CloudDownload />}
              {prefetch.isPending ? t("repositories.prefetching") : t("repositories.prefetch")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
