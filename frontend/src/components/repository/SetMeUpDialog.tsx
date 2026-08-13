import { Terminal } from "lucide-react";

import { CargoRegistryPanel } from "@/components/repository/CargoRegistryPanel";
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
  isMirror = false,
}: {
  repositoryId: string;
  isMirror?: boolean;
}) {
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
            Copia estos fragmentos en tu crate o en{" "}
            <code className="font-mono">~/.cargo</code>. El token se crea en Seguridad.
          </DialogDescription>
        </DialogHeader>
        <CargoRegistryPanel repositoryId={repositoryId} isMirror={isMirror} framed={false} />
      </DialogContent>
    </Dialog>
  );
}
