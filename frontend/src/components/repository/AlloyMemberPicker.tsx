import { useMemo } from "react";

import type { PackageEcosystemDto } from "@/api/generated/PackageEcosystemDto";
import { useRepositories } from "@/api/queries";
import { KIND_META } from "@/components/repository/RepositoryKindBadge";
import { Label } from "@/components/ui/label";

export function AlloyMemberPicker({
  ecosystem,
  selectedIds,
  onChange,
  excludeId,
}: {
  ecosystem: PackageEcosystemDto;
  selectedIds: readonly string[];
  onChange: (ids: string[]) => void;
  excludeId?: string;
}) {
  const repositoriesQuery = useRepositories();

  const eligibleMembers = useMemo(() => {
    return (repositoriesQuery.data ?? [])
      .filter(
        (repository) =>
          repository.id !== excludeId &&
          repository.ecosystem === ecosystem &&
          (repository.kind.type === "forge" || repository.kind.type === "mirror"),
      )
      .slice()
      .sort((left, right) => left.name.localeCompare(right.name));
  }, [repositoriesQuery.data, ecosystem, excludeId]);

  function toggleMember(id: string) {
    if (selectedIds.includes(id)) {
      onChange(selectedIds.filter((member) => member !== id));
      return;
    }
    onChange([...selectedIds, id]);
  }

  return (
    <div className="grid gap-2">
      <Label>Miembros</Label>
      {eligibleMembers.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          No hay repositorios Forge o Mirror de este ecosistema. Crea uno primero
          para poder agregarlo.
        </p>
      ) : (
        <ul className="max-h-40 space-y-1 overflow-y-auto rounded-md border border-border p-2">
          {eligibleMembers.map((repository) => {
            const order = selectedIds.indexOf(repository.id);
            const kindLabel = KIND_META[repository.kind.type].label;
            return (
              <li key={repository.id}>
                <label className="flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-sm hover:bg-muted">
                  <input
                    type="checkbox"
                    className="size-4 accent-primary"
                    checked={order >= 0}
                    onChange={() => toggleMember(repository.id)}
                  />
                  {order >= 0 ? (
                    <span className="flex size-5 shrink-0 items-center justify-center rounded-full bg-muted font-mono text-[10px] text-muted-foreground">
                      {order + 1}
                    </span>
                  ) : (
                    <span className="size-5 shrink-0" />
                  )}
                  <span className="min-w-0 flex-1 truncate font-medium">{repository.name}</span>
                  <span className="text-xs text-muted-foreground">{kindLabel}</span>
                </label>
              </li>
            );
          })}
        </ul>
      )}
      <p className="text-xs text-muted-foreground">
        El orden de selección es el de resolución: el primer miembro gana si hay
        la misma versión en varios.
      </p>
    </div>
  );
}
