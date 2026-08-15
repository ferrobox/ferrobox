import { Terminal } from "lucide-react";

import { CargoRegistryPanel } from "@/components/repository/CargoRegistryPanel";
import { NpmRegistryPanel } from "@/components/repository/NpmRegistryPanel";
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
  ecosystem?: "cargo" | "npm" | "pypi";
}) {
  const description =
    ecosystem === "npm" ? (
      <>
        Copia estos fragmentos en <code className="font-mono">.npmrc</code>. El token se crea en
        Seguridad.
      </>
    ) : ecosystem === "pypi" ? (
      <>
        Copia estos fragmentos en <code className="font-mono">~/.pypirc</code> o en el comando de{" "}
        <code className="font-mono">pip</code>. El token se crea en Seguridad.
      </>
    ) : (
      <>
        Copia estos fragmentos en tu crate o en <code className="font-mono">~/.cargo</code>. El
        token se crea en Seguridad.
      </>
    );

  return (
    <Dialog>
      <DialogTrigger asChild>
        <Button type="button" variant="outline" size="icon" aria-label="Configurar cliente" title="Configurar cliente">
          <Terminal />
        </Button>
      </DialogTrigger>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Configurar cliente</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        {ecosystem === "npm" ? (
          <NpmRegistryPanel repositoryId={repositoryId} kind={kind} framed={false} />
        ) : ecosystem === "pypi" ? (
          <PypiRegistryPanel repositoryId={repositoryId} kind={kind} framed={false} />
        ) : (
          <CargoRegistryPanel repositoryId={repositoryId} kind={kind} framed={false} />
        )}
      </DialogContent>
    </Dialog>
  );
}
