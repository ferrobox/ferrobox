import { useMemo, useState } from "react";
import { ChevronDown, ChevronRight, Search } from "lucide-react";
import { NavLink } from "react-router-dom";

import type { PackageEcosystemDto } from "@/api/generated/PackageEcosystemDto";
import type { RepositoryResponse } from "@/api/generated/RepositoryResponse";
import { useAuth } from "@/auth/AuthProvider";
import { canWriteArtifacts } from "@/auth/roles";
import { CreateRepositoryDialog } from "@/components/repository/CreateRepositoryDialog";
import {
  ECOSYSTEM_OPTIONS,
  ecosystemMeta,
} from "@/components/repository/EcosystemBadge";
import {
  KIND_FILTERS,
  KIND_META,
  type RepositoryStorageKind,
} from "@/components/repository/RepositoryKindBadge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

export function RepositoryBrowser({
  repositories,
  selectedId,
}: {
  repositories: readonly RepositoryResponse[];
  selectedId?: string;
}) {
  const { user } = useAuth();
  const canWrite = canWriteArtifacts(user?.role);
  const [query, setQuery] = useState("");
  const [kindFilter, setKindFilter] = useState<RepositoryStorageKind | "all">("all");
  const [collapsed, setCollapsed] = useState<ReadonlySet<PackageEcosystemDto>>(new Set());

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return repositories.filter((repository) => {
      if (kindFilter !== "all" && repository.kind.type !== kindFilter) {
        return false;
      }
      if (needle.length === 0) {
        return true;
      }
      return (
        repository.name.toLowerCase().includes(needle) ||
        repository.id.toLowerCase().includes(needle)
      );
    });
  }, [repositories, query, kindFilter]);

  const groups = useMemo(() => {
    return ECOSYSTEM_OPTIONS.map((option) => ({
      ecosystem: option.value,
      label: option.label,
      repositories: filtered
        .filter((repository) => repository.ecosystem === option.value)
        .slice()
        .sort((left, right) => left.name.localeCompare(right.name)),
    })).filter((group) => group.repositories.length > 0);
  }, [filtered]);

  function toggleGroup(ecosystem: PackageEcosystemDto) {
    setCollapsed((current) => {
      const next = new Set(current);
      if (next.has(ecosystem)) {
        next.delete(ecosystem);
      } else {
        next.add(ecosystem);
      }
      return next;
    });
  }

  return (
    <aside className="flex h-full w-80 shrink-0 flex-col border-r border-border bg-card">
      <div className="space-y-3 border-b border-border p-4">
        <div className="flex items-center justify-between gap-2">
          <h2 className="text-sm font-semibold text-foreground">Artefactos</h2>
          {canWrite ? <CreateRepositoryDialog compact /> : null}
        </div>
        <div className="relative">
          <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Buscar repositorio…"
            className="pl-8"
            aria-label="Buscar repositorio"
          />
        </div>
        <div className="flex flex-wrap gap-1">
          <FilterChip
            active={kindFilter === "all"}
            onClick={() => setKindFilter("all")}
            label="Todos"
          />
          {KIND_FILTERS.map((kind) => (
            <FilterChip
              key={kind}
              active={kindFilter === kind}
              onClick={() => setKindFilter(kind)}
              label={KIND_META[kind].label}
            />
          ))}
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto py-2">
        {repositories.length === 0 ? (
          <p className="px-4 py-8 text-center text-sm text-muted-foreground">
            Aún no hay repositorios. Crea uno para empezar.
          </p>
        ) : groups.length === 0 ? (
          <p className="px-4 py-8 text-center text-sm text-muted-foreground">
            Ningún repositorio coincide con el filtro.
          </p>
        ) : (
          groups.map((group) => {
            const meta = ecosystemMeta(group.ecosystem);
            const Icon = meta.icon;
            const isCollapsed = collapsed.has(group.ecosystem);

            return (
              <section key={group.ecosystem} className="mb-1">
                <button
                  type="button"
                  onClick={() => toggleGroup(group.ecosystem)}
                  className="flex w-full items-center gap-2 px-4 py-1.5 text-left text-xs font-semibold tracking-wide text-muted-foreground uppercase hover:text-foreground"
                >
                  {isCollapsed ? (
                    <ChevronRight className="size-3.5" />
                  ) : (
                    <ChevronDown className="size-3.5" />
                  )}
                  <Icon className="size-3.5" />
                  {group.label}
                  <span className="font-normal tabular-nums">
                    {group.repositories.length}
                  </span>
                </button>
                {isCollapsed ? null : (
                  <ul>
                    {group.repositories.map((repository) => {
                      const kind = KIND_META[repository.kind.type];
                      const KindIcon = kind.icon;

                      return (
                        <li key={repository.id}>
                          <NavLink
                            to={`/repositories/${repository.id}`}
                            title={kind.description}
                            className={({ isActive }) =>
                              cn(
                                "flex items-start gap-2.5 px-4 py-2 text-sm transition-colors",
                                isActive || selectedId === repository.id
                                  ? "bg-sidebar-accent text-foreground"
                                  : "text-foreground/80 hover:bg-muted/60",
                              )
                            }
                          >
                            <KindIcon className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />
                            <span className="min-w-0">
                              <span className="block truncate font-medium">
                                {repository.name}
                              </span>
                              <span className="block text-[11px] text-muted-foreground">
                                {kind.label} · {kind.domain}
                              </span>
                            </span>
                          </NavLink>
                        </li>
                      );
                    })}
                  </ul>
                )}
              </section>
            );
          })
        )}
      </div>
    </aside>
  );
}

function FilterChip({
  active,
  label,
  onClick,
}: {
  active: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <Button
      type="button"
      size="xs"
      variant={active ? "default" : "outline"}
      onClick={onClick}
    >
      {label}
    </Button>
  );
}
