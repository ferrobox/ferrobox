import { Boxes } from "lucide-react";

import { useAuth } from "@/auth/AuthProvider";
import { canWriteArtifacts } from "@/auth/roles";
import { CreateRepositoryDialog } from "@/components/repository/CreateRepositoryDialog";

export function RepositoriesPage() {
  const { user } = useAuth();
  const canWrite = canWriteArtifacts(user?.role);

  return (
    <div className="flex h-full flex-col items-center justify-center gap-4 text-center">
      <div className="flex size-12 items-center justify-center rounded-full bg-muted">
        <Boxes className="size-6 text-muted-foreground" />
      </div>
      <div className="max-w-md space-y-1">
        <h1 className="text-xl font-semibold text-foreground">Repositorios</h1>
        <p className="text-sm text-muted-foreground">
          Elige un repositorio en el árbol de la izquierda. Están agrupados por
          ecosistema (Cargo, npm…) y etiquetados como Local, Remoto o Virtual,
          igual que en Artifactory.
        </p>
      </div>
      {canWrite ? <CreateRepositoryDialog /> : null}
    </div>
  );
}
