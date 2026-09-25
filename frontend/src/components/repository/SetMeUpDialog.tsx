import { Terminal } from "lucide-react";
import { useTranslation } from "react-i18next";

import { CargoRegistryPanel } from "@/components/repository/CargoRegistryPanel";
import { ConanRegistryPanel } from "@/components/repository/ConanRegistryPanel";
import { HelmRegistryPanel } from "@/components/repository/HelmRegistryPanel";
import { MavenRegistryPanel } from "@/components/repository/MavenRegistryPanel";
import { NpmRegistryPanel } from "@/components/repository/NpmRegistryPanel";
import { OciRegistryPanel } from "@/components/repository/OciRegistryPanel";
import { PypiRegistryPanel } from "@/components/repository/PypiRegistryPanel";
import type { RepositoryStorageKind } from "@/components/repository/RepositoryKindBadge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";

export function SetMeUpDialog({
  repositoryId,
  kind = "forge",
  ecosystem = "cargo",
}: {
  repositoryId: string;
  kind?: RepositoryStorageKind;
  ecosystem?: "cargo" | "npm" | "pypi" | "oci" | "helm" | "conan" | "maven";
}) {
  const { t } = useTranslation();
  const description = t(`setup.${ecosystem}`);

  return (
    <Dialog>
      <DialogTrigger asChild>
        <Button type="button" variant="outline" size="icon" aria-label={t("setup.aria")} title={t("setup.aria")}>
          <Terminal />
        </Button>
      </DialogTrigger>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t("setup.title")}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        {ecosystem === "npm" ? (
          <NpmRegistryPanel repositoryId={repositoryId} kind={kind} framed={false} />
        ) : ecosystem === "pypi" ? (
          <PypiRegistryPanel repositoryId={repositoryId} kind={kind} framed={false} />
        ) : ecosystem === "oci" ? (
          <OciRegistryPanel repositoryId={repositoryId} kind={kind} framed={false} />
        ) : ecosystem === "helm" ? (
          <HelmRegistryPanel repositoryId={repositoryId} kind={kind} framed={false} />
        ) : ecosystem === "conan" ? (
          <ConanRegistryPanel repositoryId={repositoryId} kind={kind} framed={false} />
        ) : ecosystem === "maven" ? (
          <MavenRegistryPanel repositoryId={repositoryId} kind={kind} framed={false} />
        ) : (
          <CargoRegistryPanel repositoryId={repositoryId} kind={kind} framed={false} />
        )}
      </DialogContent>
    </Dialog>
  );
}
