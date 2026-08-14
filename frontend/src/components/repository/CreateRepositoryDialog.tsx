import { type FormEvent, useState } from "react";
import { Loader2, Plus } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import type { CreateRepositoryKindDto } from "@/api/generated/CreateRepositoryKindDto";
import type { PackageEcosystemDto } from "@/api/generated/PackageEcosystemDto";
import { useCreateRepository } from "@/api/queries";
import { AlloyMemberPicker } from "@/components/repository/AlloyMemberPicker";
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
const MIRROR_ECOSYSTEMS: readonly PackageEcosystemDto[] = ["cargo", "npm"];
const DEFAULT_UPSTREAM = {
  cargo: "https://index.crates.io/",
  npm: "https://registry.npmjs.org/",
} as const;

type KindChoice = "forge" | "mirror" | "alloy";

function isMirrorEcosystem(
  ecosystem: PackageEcosystemDto,
): ecosystem is "cargo" | "npm" {
  return MIRROR_ECOSYSTEMS.includes(ecosystem);
}

function defaultUpstreamFor(ecosystem: PackageEcosystemDto): string {
  return ecosystem === "npm" ? DEFAULT_UPSTREAM.npm : DEFAULT_UPSTREAM.cargo;
}

function isKnownDefaultUpstream(value: string): boolean {
  const trimmed = value.trim();
  return (
    trimmed.length === 0 ||
    trimmed === DEFAULT_UPSTREAM.cargo ||
    trimmed === DEFAULT_UPSTREAM.npm
  );
}

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
      if (!isMirrorEcosystem(ecosystem)) {
        setValidationError("Los Mirror están disponibles para Cargo y npm.");
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
              registro externo (crates.io o registry.npmjs.org). Un Alloy agrega
              Forges y/o Mirrors del mismo ecosistema en una sola URL.
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
                    const nextEcosystem = isMirrorEcosystem(ecosystem) ? ecosystem : "cargo";
                    setEcosystem(nextEcosystem);
                    if (isKnownDefaultUpstream(upstream)) {
                      setUpstream(defaultUpstreamFor(nextEcosystem));
                    }
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
                  <SelectItem value="mirror">Mirror</SelectItem>
                  <SelectItem value="alloy">Alloy</SelectItem>
                </SelectContent>
              </Select>
            </div>

            <div className="grid gap-2">
              <Label htmlFor="repository-ecosystem">Ecosistema de paquetes</Label>
              <Select
                value={ecosystem}
                onValueChange={(value) => {
                  const next = value as PackageEcosystemDto;
                  setEcosystem(next);
                  setMemberIds([]);
                  if (kind === "mirror" && isKnownDefaultUpstream(upstream)) {
                    setUpstream(defaultUpstreamFor(next));
                  }
                }}
              >
                <SelectTrigger id="repository-ecosystem" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {(kind === "mirror"
                    ? ECOSYSTEM_OPTIONS.filter((option) => isMirrorEcosystem(option.value))
                    : ECOSYSTEM_OPTIONS
                  ).map((option) => (
                    <SelectItem key={option.value} value={option.value}>
                      {option.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            {kind === "mirror" ? (
              <div className="grid gap-2">
                <Label htmlFor="repository-upstream">
                  {ecosystem === "npm" ? "Upstream (registro npm)" : "Upstream (índice disperso)"}
                </Label>
                <Input
                  id="repository-upstream"
                  placeholder={defaultUpstreamFor(ecosystem)}
                  value={upstream}
                  onChange={(event) => setUpstream(event.target.value)}
                />
                <p className="text-xs text-muted-foreground">
                  {ecosystem === "npm" ? (
                    <>
                      URL base del registro npm (por ejemplo{" "}
                      <code className="font-mono">https://registry.npmjs.org/</code>).
                    </>
                  ) : (
                    <>
                      URL base del índice sparse (sin el prefijo <code>sparse+</code>).
                    </>
                  )}
                </p>
              </div>
            ) : null}

            {kind === "alloy" ? (
              <AlloyMemberPicker
                ecosystem={ecosystem}
                selectedIds={memberIds}
                onChange={setMemberIds}
              />
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
