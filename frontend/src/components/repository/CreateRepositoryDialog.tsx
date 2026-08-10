import { type FormEvent, useState } from "react";
import { Loader2, Plus } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useCreateRepository } from "@/api/queries";
import type { PackageEcosystemDto } from "@/api/generated/PackageEcosystemDto";
import { ECOSYSTEM_OPTIONS } from "@/components/repository/EcosystemBadge";
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

const NAME_PATTERN = /^[A-Za-z0-9_-]+$/;

export function CreateRepositoryDialog() {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [ecosystem, setEcosystem] = useState<PackageEcosystemDto>("generic");
  const [validationError, setValidationError] = useState<string | null>(null);
  const navigate = useNavigate();
  const mutation = useCreateRepository();

  function resetAndClose() {
    setOpen(false);
    setName("");
    setEcosystem("generic");
    setValidationError(null);
    mutation.reset();
  }

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    setValidationError(null);

    const trimmed = name.trim();
    if (trimmed.length === 0) {
      setValidationError("El nombre no puede estar vacío.");
      return;
    }
    if (!NAME_PATTERN.test(trimmed)) {
      setValidationError(
        "Solo se permiten letras, números, guiones ('-') y guiones bajos ('_').",
      );
      return;
    }

    mutation.mutate(
      { name: trimmed, ecosystem },
      {
        onSuccess: (response) => {
          toast.success(`Repositorio «${trimmed}» creado correctamente.`);
          resetAndClose();
          navigate(`/repositories/${response.id}`);
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
        if (!next) {
          resetAndClose();
        } else {
          setOpen(true);
        }
      }}
    >
      <DialogTrigger asChild>
        <Button>
          <Plus />
          Nuevo repositorio
        </Button>
      </DialogTrigger>
      <DialogContent>
        <form onSubmit={handleSubmit}>
          <DialogHeader>
            <DialogTitle>Crear repositorio</DialogTitle>
            <DialogDescription>
              Se creará como un repositorio Forge: FerroBox será la fuente de verdad del
              contenido publicado directamente en él.
            </DialogDescription>
          </DialogHeader>

          <div className="grid gap-4 py-4">
            <div className="grid gap-2">
              <Label htmlFor="repository-name">Nombre</Label>
              <Input
                id="repository-name"
                autoFocus
                placeholder="mi-repositorio-cargo"
                value={name}
                onChange={(event) => setName(event.target.value)}
              />
            </div>

            <div className="grid gap-2">
              <Label htmlFor="repository-ecosystem">Ecosistema de paquetes</Label>
              <Select
                value={ecosystem}
                onValueChange={(value) => setEcosystem(value as PackageEcosystemDto)}
              >
                <SelectTrigger id="repository-ecosystem" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {ECOSYSTEM_OPTIONS.map((option) => (
                    <SelectItem key={option.value} value={option.value}>
                      {option.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            {(validationError ?? serverError) ? (
              <p className="text-sm text-destructive">{validationError ?? serverError}</p>
            ) : null}
          </div>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={resetAndClose}>
              Cancelar
            </Button>
            <Button type="submit" disabled={mutation.isPending}>
              {mutation.isPending ? <Loader2 className="animate-spin" /> : null}
              Crear repositorio
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
