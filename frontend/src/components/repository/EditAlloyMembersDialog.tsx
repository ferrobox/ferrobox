import { type FormEvent, useState } from "react";
import { Loader2, Pencil } from "lucide-react";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import type { PackageEcosystemDto } from "@/api/generated/PackageEcosystemDto";
import { useUpdateAlloyMembers } from "@/api/queries";
import { AlloyMemberPicker } from "@/components/repository/AlloyMemberPicker";
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

export function EditAlloyMembersDialog({
  repositoryId,
  ecosystem,
  members,
}: {
  repositoryId: string;
  ecosystem: PackageEcosystemDto;
  members: readonly string[];
}) {
  const [open, setOpen] = useState(false);
  const [memberIds, setMemberIds] = useState<string[]>([...members]);
  const [validationError, setValidationError] = useState<string | null>(null);
  const mutation = useUpdateAlloyMembers(repositoryId);

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    setValidationError(null);

    if (memberIds.length === 0) {
      setValidationError("Un Alloy necesita al menos un repositorio Forge o Mirror.");
      return;
    }

    mutation.mutate(
      { members: memberIds },
      {
        onSuccess: () => {
          toast.success("Miembros del Alloy actualizados.");
          setOpen(false);
        },
      },
    );
  }

  const serverError =
    mutation.error instanceof ApiError ? mutation.error.message : mutation.error?.message;

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (next) {
          setMemberIds([...members]);
          setValidationError(null);
          mutation.reset();
        }
        setOpen(next);
      }}
    >
      <DialogTrigger asChild>
        <Button type="button" variant="outline" size="icon" aria-label="Editar miembros" title="Editar miembros">
          <Pencil />
        </Button>
      </DialogTrigger>
      <DialogContent>
        <form onSubmit={handleSubmit}>
          <DialogHeader>
            <DialogTitle>Editar miembros</DialogTitle>
            <DialogDescription>
              Cambia qué Forges y Mirrors agrega este Alloy, y en qué orden se
              resuelven las lecturas.
            </DialogDescription>
          </DialogHeader>

          <div className="grid gap-4 py-4">
            <AlloyMemberPicker
              ecosystem={ecosystem}
              selectedIds={memberIds}
              onChange={setMemberIds}
              excludeId={repositoryId}
            />
            {(validationError ?? serverError) ? (
              <p className="text-sm text-destructive">{validationError ?? serverError}</p>
            ) : null}
          </div>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => setOpen(false)}>
              Cancelar
            </Button>
            <Button type="submit" disabled={mutation.isPending}>
              {mutation.isPending ? <Loader2 className="animate-spin" /> : null}
              Guardar
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
