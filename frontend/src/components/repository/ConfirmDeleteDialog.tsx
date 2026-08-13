import { type ReactNode, useState } from "react";
import { Loader2 } from "lucide-react";

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

export function ConfirmDeleteDialog({
  trigger,
  title,
  description,
  confirmLabel = "Eliminar",
  pending = false,
  onConfirm,
}: {
  trigger: ReactNode;
  title: string;
  description: string;
  confirmLabel?: string;
  pending?: boolean;
  onConfirm: () => Promise<void>;
}) {
  const [open, setOpen] = useState(false);

  async function handleConfirm() {
    try {
      await onConfirm();
      setOpen(false);
    } catch {
      // El llamador muestra el error; el diálogo permanece abierto.
    }
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>{trigger}</DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => setOpen(false)}>
            Cancelar
          </Button>
          <Button
            type="button"
            variant="destructive"
            disabled={pending}
            onClick={() => void handleConfirm()}
          >
            {pending ? <Loader2 className="animate-spin" /> : null}
            {confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
