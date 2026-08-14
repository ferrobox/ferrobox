import { Terminal } from "lucide-react";

import { CargoRegistryPanel } from "@/components/repository/CargoRegistryPanel";
import { NpmRegistryPanel } from "@/components/repository/NpmRegistryPanel";
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
  ecosystem?: "cargo" | "npm";
}) {
  const isNpm = ecosystem === "npm";

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
          <DialogDescription>
            {isNpm ? (
              <>
                Copia estos fragmentos en <code className="font-mono">.npmrc</code>. El token se
                crea en Seguridad.
              </>
            ) : (
              <>
                Copia estos fragmentos en tu crate o en{" "}
                <code className="font-mono">~/.cargo</code>. El token se crea en Seguridad.
              </>
            )}
          </DialogDescription>
        </DialogHeader>
        {isNpm ? (
          <NpmRegistryPanel repositoryId={repositoryId} kind={kind} framed={false} />
        ) : (
          <CargoRegistryPanel repositoryId={repositoryId} kind={kind} framed={false} />
        )}
      </DialogContent>
    </Dialog>
  );
}
