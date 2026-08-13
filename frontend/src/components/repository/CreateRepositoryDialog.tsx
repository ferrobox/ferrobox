import { type FormEvent, useMemo, useState } from "react";
import { Loader2, Plus } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import type { CreateRepositoryKindDto } from "@/api/generated/CreateRepositoryKindDto";
import type { PackageEcosystemDto } from "@/api/generated/PackageEcosystemDto";
import { useCreateRepository, useRepositories } from "@/api/queries";
import { ECOSYSTEM_OPTIONS } from "@/components/repository/EcosystemBadge";
import { KIND_META } from "@/components/repository/RepositoryKindBadge";
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

type KindChoice = "forge" | "mirror" | "alloy";

export function CreateRepositoryDialog({ compact = false }: { compact?: boolean }) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [ecosystem, setEcosystem] = useState<PackageEcosystemDto>("generic");
  const [kind, setKind] = useState<KindChoice>("forge");
  const [upstream, setUpstream] = useState("https://index.crates.io/");
  const [memberIds, setMemberIds] = useState<string[]>([]);
  const [validationError, setValidationError] = useState<string | null>(null);
  const navigate = useNavigate();
  const mutation = useCreateRepository();
  const repositoriesQuery = useRepositories();

  const eligibleMembers = useMemo(() => {
    return (repositoriesQuery.data ?? []).filter(
      (repository) =>
        repository.ecosystem === ecosystem &&
        (repository.kind.type === "forge" || repository.kind.type === "mirror"),
    );
  }, [repositoriesQuery.data, ecosystem]);

  function resetAndClose() {
    setOpen(false);
    setName("");
    setEcosystem("generic");
    setKind("forge");
    setUpstream("https://index.crates.io/");
    setMemberIds([]);
    setValidationError(null);
    mutation.reset();
  }

  function toggleMember(id: string) {
    setMemberIds((current) =>
      current.includes(id) ? current.filter((member) => member !== id) : [...current, id],
    );
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

    if (kind === "mirror") {
      if (ecosystem !== "cargo") {
        setValidationError("Los Mirror solo están disponibles para el ecosistema Cargo.");
        return;
      }
      try {
        void new URL(upstream.trim());
      } catch {
        setValidationError("La URL upstream no es válida.");
        return;
      }
    }

    if (kind === "alloy" && memberIds.length === 0) {
      setValidationError("Un Alloy necesita al menos un repositorio Forge o Mirror.");
      return;
    }

    const kindPayload: CreateRepositoryKindDto =
      kind === "mirror"
        ? { type: "mirror", upstream: upstream.trim() }
        : kind === "alloy"
          ? { type: "alloy", members: memberIds }
          : { type: "forge" };

    mutation.mutate(
      { name: trimmed, ecosystem, kind: kindPayload },
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
        <Button
          size={compact ? "icon-sm" : "default"}
          aria-label={compact ? "Nuevo repositorio" : undefined}
          title={compact ? "Nuevo repositorio" : undefined}
        >
          <Plus />
          {compact ? null : "Nuevo repositorio"}
        </Button>
      </DialogTrigger>
      <DialogContent>
        <form onSubmit={handleSubmit}>
          <DialogHeader>
            <DialogTitle>Crear repositorio</DialogTitle>
            <DialogDescription>
              Un Forge guarda artefactos que publicas tú. Un Mirror cachea un
              índice Cargo externo como crates.io. Un Alloy agrega Forges y/o
              Mirrors del mismo ecosistema en una sola URL.
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
              <Label htmlFor="repository-kind">Tipo</Label>
              <Select
                value={kind}
                onValueChange={(value) => {
                  const next = value as KindChoice;
                  setKind(next);
                  if (next === "mirror") {
                    setEcosystem("cargo");
                    setMemberIds([]);
                  }
                  if (next !== "alloy") {
                    setMemberIds([]);
                  }
                }}
              >
                <SelectTrigger id="repository-kind" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="forge">Forge</SelectItem>
                  <SelectItem value="mirror">Mirror (Cargo)</SelectItem>
                  <SelectItem value="alloy">Alloy</SelectItem>
                </SelectContent>
              </Select>
            </div>

            <div className="grid gap-2">
              <Label htmlFor="repository-ecosystem">Ecosistema de paquetes</Label>
              <Select
                value={ecosystem}
                onValueChange={(value) => {
                  setEcosystem(value as PackageEcosystemDto);
                  setMemberIds([]);
                }}
                disabled={kind === "mirror"}
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

            {kind === "mirror" ? (
              <div className="grid gap-2">
                <Label htmlFor="repository-upstream">Upstream (índice disperso)</Label>
                <Input
                  id="repository-upstream"
                  placeholder="https://index.crates.io/"
                  value={upstream}
                  onChange={(event) => setUpstream(event.target.value)}
                />
                <p className="text-xs text-muted-foreground">
                  URL base del índice sparse (sin el prefijo <code>sparse+</code>).
                </p>
              </div>
            ) : null}

            {kind === "alloy" ? (
              <div className="grid gap-2">
                <Label>Miembros</Label>
                {eligibleMembers.length === 0 ? (
                  <p className="text-sm text-muted-foreground">
                    No hay repositorios Forge o Mirror de este ecosistema. Crea
                    uno primero para poder agregarlo.
                  </p>
                ) : (
                  <ul className="max-h-40 space-y-1 overflow-y-auto rounded-md border border-border p-2">
                    {eligibleMembers.map((repository) => {
                      const checked = memberIds.includes(repository.id);
                      const kindLabel = KIND_META[repository.kind.type].label;
                      return (
                        <li key={repository.id}>
                          <label className="flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-sm hover:bg-muted">
                            <input
                              type="checkbox"
                              className="size-4 accent-primary"
                              checked={checked}
                              onChange={() => toggleMember(repository.id)}
                            />
                            <span className="min-w-0 flex-1 truncate font-medium">
                              {repository.name}
                            </span>
                            <span className="text-xs text-muted-foreground">{kindLabel}</span>
                          </label>
                        </li>
                      );
                    })}
                  </ul>
                )}
                <p className="text-xs text-muted-foreground">
                  El orden de selección es el de resolución: el primer miembro
                  gana si hay la misma versión en varios.
                </p>
              </div>
            ) : null}

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
