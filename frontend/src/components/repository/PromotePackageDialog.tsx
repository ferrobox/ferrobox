import { type FormEvent, useMemo, useState } from "react";
import { ArrowRightLeft, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import type { ArtifactResponse } from "@/api/generated/ArtifactResponse";
import type { PackageEcosystemDto } from "@/api/generated/PackageEcosystemDto";
import { usePromotePackage, useRepositories } from "@/api/queries";
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
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

function writableForgeTargets(
  repositories: ReadonlyArray<{
    id: string;
    name: string;
    ecosystem: PackageEcosystemDto;
    kind: { type: string };
    access: "read" | "write";
  }>,
  sourceRepositoryId: string,
  ecosystem: PackageEcosystemDto,
) {
  return repositories.filter(
    (repository) =>
      repository.id !== sourceRepositoryId &&
      repository.kind.type === "forge" &&
      repository.ecosystem === ecosystem &&
      repository.access === "write",
  );
}

export function PromotePackageDialog({
  sourceRepositoryId,
  ecosystem,
  artifact,
  versionLabel,
}: {
  sourceRepositoryId: string;
  ecosystem: PackageEcosystemDto;
  artifact: ArtifactResponse;
  versionLabel: string;
}) {
  const { t } = useTranslation();
  const { data: repositories } = useRepositories();
  const promote = usePromotePackage(sourceRepositoryId);
  const [open, setOpen] = useState(false);
  const [targetId, setTargetId] = useState("");

  const targets = useMemo(
    () => writableForgeTargets(repositories ?? [], sourceRepositoryId, ecosystem),
    [repositories, sourceRepositoryId, ecosystem],
  );

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!targetId) {
      toast.error(t("promote.chooseForge"));
      return;
    }
    try {
      const outcome = await promote.mutateAsync({
        targetRepositoryId: targetId,
        payload: {
          target_repository_id: targetId,
          name: artifact.name ?? undefined,
          version: artifact.version ?? undefined,
          artifact_id: artifact.name && artifact.version ? undefined : artifact.id,
          preserve_yanked: false,
        },
      });
      const targetName =
        targets.find((repository) => repository.id === targetId)?.name ?? targetId;
      setOpen(false);
      setTargetId("");
      toast.success(
        t("promote.copied", {
          name: targetName,
          count: outcome.artifacts_copied,
          kind:
            outcome.artifacts_copied === 1
              ? t("promote.artifact")
              : t("promote.artifacts"),
        }),
      );
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("promote.failed"));
    }
  }

  if (targets.length === 0) {
    return null;
  }

  const subject =
    artifact.name && artifact.version
      ? `${artifact.name} ${versionLabel}`
      : artifact.id;

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) {
          setTargetId("");
        }
      }}
    >
      <DialogTrigger asChild>
        <Button variant="ghost" size="sm">
          <ArrowRightLeft />
          {t("promote.action")}
        </Button>
      </DialogTrigger>
      <DialogContent>
        <form onSubmit={(event) => void onSubmit(event)}>
          <DialogHeader>
            <DialogTitle>{t("promote.title")}</DialogTitle>
            <DialogDescription>{t("promote.hint", { subject })}</DialogDescription>
          </DialogHeader>
          <div className="grid gap-4 py-4">
            <div className="grid gap-2">
              <Label htmlFor="promote-target">{t("promote.target")}</Label>
              <Select value={targetId || undefined} onValueChange={setTargetId}>
                <SelectTrigger id="promote-target" className="w-full">
                  <SelectValue placeholder={t("promote.selectForge")} />
                </SelectTrigger>
                <SelectContent>
                  {targets.map((repository) => (
                    <SelectItem key={repository.id} value={repository.id}>
                      {repository.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>
          <DialogFooter>
            <Button type="submit" disabled={!targetId || promote.isPending}>
              {promote.isPending ? <Loader2 className="animate-spin" /> : <ArrowRightLeft />}
              {t("promote.action")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
