import { type FormEvent, useState } from "react";
import { Loader2, Plus } from "lucide-react";
import { useTranslation } from "react-i18next";
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
const MIRROR_ECOSYSTEMS: readonly PackageEcosystemDto[] = [
  "cargo",
  "npm",
  "pypi",
  "oci",
  "helm",
  "maven",
  "nuget",
];
const DEFAULT_UPSTREAM = {
  cargo: "https://index.crates.io/",
  npm: "https://registry.npmjs.org/",
  pypi: "https://pypi.org/simple/",
  oci: "https://registry-1.docker.io",
  helm: "https://registry-1.docker.io",
  maven: "https://repo1.maven.org/maven2/",
  nuget: "https://api.nuget.org/v3/index.json",
} as const;

type KindChoice = "forge" | "mirror" | "alloy";

function isMirrorEcosystem(
  ecosystem: PackageEcosystemDto,
): ecosystem is "cargo" | "npm" | "pypi" | "oci" | "helm" | "maven" | "nuget" {
  return MIRROR_ECOSYSTEMS.includes(ecosystem);
}

function defaultUpstreamFor(ecosystem: PackageEcosystemDto): string {
  if (ecosystem === "npm") {
    return DEFAULT_UPSTREAM.npm;
  }
  if (ecosystem === "pypi") {
    return DEFAULT_UPSTREAM.pypi;
  }
  if (ecosystem === "oci") {
    return DEFAULT_UPSTREAM.oci;
  }
  if (ecosystem === "helm") {
    return DEFAULT_UPSTREAM.helm;
  }
  if (ecosystem === "maven") {
    return DEFAULT_UPSTREAM.maven;
  }
  if (ecosystem === "nuget") {
    return DEFAULT_UPSTREAM.nuget;
  }
  return DEFAULT_UPSTREAM.cargo;
}

function isKnownDefaultUpstream(value: string): boolean {
  const trimmed = value.trim();
  return (
    trimmed.length === 0 ||
    trimmed === DEFAULT_UPSTREAM.cargo ||
    trimmed === DEFAULT_UPSTREAM.npm ||
    trimmed === DEFAULT_UPSTREAM.pypi ||
    trimmed === DEFAULT_UPSTREAM.oci ||
    trimmed === DEFAULT_UPSTREAM.helm ||
    trimmed === DEFAULT_UPSTREAM.maven ||
    trimmed === DEFAULT_UPSTREAM.nuget
  );
}

export function CreateRepositoryDialog({ compact = false }: { compact?: boolean }) {
  const { t } = useTranslation();
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
      setValidationError(t("repositories.nameEmpty"));
      return;
    }
    if (!NAME_PATTERN.test(trimmed)) {
      setValidationError(t("repositories.nameChars"));
      return;
    }

    if (kind === "mirror") {
      if (!isMirrorEcosystem(ecosystem)) {
        setValidationError(t("repositories.mirrorEcosystems"));
        return;
      }
      try {
        void new URL(upstream.trim());
      } catch {
        setValidationError(t("repositories.upstreamInvalid"));
        return;
      }
    }

    if (kind === "alloy" && memberIds.length === 0) {
      setValidationError(t("repositories.alloyNeedsMember"));
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
          toast.success(t("repositories.created", { name: trimmed }));
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
          aria-label={compact ? t("repositories.new") : undefined}
          title={compact ? t("repositories.new") : undefined}
        >
          <Plus />
          {compact ? null : t("repositories.new")}
        </Button>
      </DialogTrigger>
      <DialogContent>
        <form onSubmit={handleSubmit}>
          <DialogHeader>
            <DialogTitle>{t("repositories.create")}</DialogTitle>
            <DialogDescription>{t("repositories.createHint")}</DialogDescription>
          </DialogHeader>

          <div className="grid gap-4 py-4">
            <div className="grid gap-2">
              <Label htmlFor="repository-name">{t("repositories.name")}</Label>
              <Input
                id="repository-name"
                autoFocus
                placeholder={t("repositories.namePlaceholder")}
                value={name}
                onChange={(event) => setName(event.target.value)}
              />
            </div>

            <div className="grid gap-2">
              <Label htmlFor="repository-kind">{t("repositories.kind")}</Label>
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
              <Label htmlFor="repository-ecosystem">{t("repositories.ecosystem")}</Label>
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
                  {ecosystem === "npm"
                    ? t("upstream.npm")
                    : ecosystem === "pypi"
                      ? t("upstream.pypi")
                      : ecosystem === "oci"
                        ? t("upstream.oci")
                        : ecosystem === "helm"
                          ? t("upstream.helm")
                          : ecosystem === "maven"
                            ? t("upstream.maven")
                            : ecosystem === "nuget"
                              ? t("upstream.nuget")
                              : t("upstream.cargo")}
                </Label>
                <Input
                  id="repository-upstream"
                  placeholder={defaultUpstreamFor(ecosystem)}
                  value={upstream}
                  onChange={(event) => setUpstream(event.target.value)}
                />
                <p className="text-xs text-muted-foreground">
                  {ecosystem === "npm"
                    ? t("upstream.npmHint")
                    : ecosystem === "pypi"
                      ? t("upstream.pypiHint")
                      : ecosystem === "oci"
                        ? t("upstream.ociHint")
                        : ecosystem === "helm"
                          ? t("upstream.helmHint")
                          : ecosystem === "maven"
                            ? t("upstream.mavenHint")
                            : ecosystem === "nuget"
                              ? t("upstream.nugetHint")
                              : t("upstream.cargoHint")}
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
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={mutation.isPending}>
              {mutation.isPending ? <Loader2 className="animate-spin" /> : null}
              {mutation.isPending ? t("repositories.creating") : t("repositories.create")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
