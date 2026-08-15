import { Check, Copy } from "lucide-react";
import { useState } from "react";

import { useSettings } from "@/api/queries";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";

function CopyableCodeBlock({ code }: { code: string }) {
  const [copied, setCopied] = useState(false);

  async function handleCopy() {
    await navigator.clipboard.writeText(code);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  return (
    <div className="group relative">
      <pre className="overflow-x-auto rounded-md bg-muted px-4 py-3 font-mono text-xs text-foreground">
        {code}
      </pre>
      <Button
        variant="ghost"
        size="icon"
        className="absolute top-1.5 right-1.5 size-7 opacity-0 transition-opacity group-hover:opacity-100"
        onClick={() => void handleCopy()}
        aria-label="Copiar al portapapeles"
      >
        {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
      </Button>
    </div>
  );
}

export function ConanRegistryPanel({
  repositoryId,
  kind = "forge",
  framed = true,
}: {
  repositoryId: string;
  kind?: "forge" | "mirror" | "alloy";
  framed?: boolean;
}) {
  const { data, isPending, isError } = useSettings();

  if (isPending) {
    return <Skeleton className="h-40 w-full rounded-lg" />;
  }

  if (isError || !data) {
    return (
      <p className="text-sm text-muted-foreground">
        No se pudo obtener la URL pública de la instancia.
      </p>
    );
  }

  const remoteUrl = `${data.public_base_url.replace(/\/$/, "")}/conan/${repositoryId}`;
  const isMirror = kind === "mirror";
  const isAlloy = kind === "alloy";
  const readOnly = isMirror || isAlloy;

  const introTitle = isAlloy ? "Alloy Conan" : isMirror ? "Mirror Conan" : "Registro Conan";
  const introBody = isAlloy
    ? "Este Alloy agrega Forges Conan en una sola URL. conan install resuelve contra los miembros, en orden. No acepta conan upload: publica en un Forge miembro."
    : isMirror
      ? "El Mirror Conan todavía no está implementado. Usa un Forge o un Alloy de Forges."
      : "Este repositorio implementa la API v2 de Conan (revisiones). conan upload y conan install hablan con /conan/<UUID>/.";

  const remote = `conan remote add ferrobox ${remoteUrl}`;
  const login = `conan remote login ferrobox __token__`;
  const upload = `conan new cmake_lib -d name=hello -d version=0.1\nconan create .\nconan upload "hello/0.1" -r ferrobox -c`;
  const install = `conan install --requires=hello/0.1 -r ferrobox`;

  const body = (
    <div className="space-y-5">
      <div>
        <h3 className="text-sm font-semibold text-foreground">{introTitle}</h3>
        <p className="mt-1 text-sm text-muted-foreground">{introBody}</p>
      </div>

      <div className="space-y-2">
        <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          1. Añade el remoto
        </p>
        <CopyableCodeBlock code={remote} />
        <p className="text-xs text-muted-foreground">
          Copia el UUID completo (8-4-4-4-12). La URL del remoto es{" "}
          <code className="font-mono">{remoteUrl}</code>.
        </p>
      </div>

      {readOnly ? null : (
        <div className="space-y-2">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            2. Login (token de Seguridad)
          </p>
          <CopyableCodeBlock code={login} />
          <p className="text-xs text-muted-foreground">
            Usuario <code className="font-mono">__token__</code>. Cuando pida la contraseña, pega el
            token de Seguridad.
          </p>
        </div>
      )}

      <div className="space-y-2">
        <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          {readOnly ? "2. Instala" : "3. Crea, publica e instala"}
        </p>
        <CopyableCodeBlock code={readOnly ? install : `${upload}\n${install}`} />
      </div>
    </div>
  );

  if (!framed) {
    return body;
  }

  return <div className="rounded-lg border border-border p-5">{body}</div>;
}
