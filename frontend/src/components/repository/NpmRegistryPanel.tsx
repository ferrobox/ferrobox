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

export function NpmRegistryPanel({
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

  const registryUrl = `${data.public_base_url.replace(/\/$/, "")}/npm/${repositoryId}/`;
  const authHost = registryUrl.replace(/^https?:\/\//, "");
  const isMirror = kind === "mirror";
  const isAlloy = kind === "alloy";
  const readOnly = isMirror || isAlloy;

  const introTitle = isAlloy
    ? "Alloy npm"
    : isMirror
      ? "Mirror npm"
      : "Registro npm";
  const quarantineDays = data.mirror_quarantine_days;
  const showQuarantine = (isMirror || isAlloy) && quarantineDays > 0;
  const introBody = isAlloy
    ? "Este Alloy agrega Forges y/o Mirrors npm en una sola URL. npm install resuelve contra los miembros, en orden. No acepta npm publish: publica en un Forge miembro."
    : isMirror
      ? "Este Mirror cachea paquetes del upstream la primera vez que npm install los resuelve o descarga. No acepta npm publish ni yank."
      : "Este repositorio implementa el protocolo de registro de npm: npm publish y npm install funcionan de forma nativa.";

  const body = (
    <div className="space-y-5">
      <div>
        <h3 className="text-sm font-semibold text-foreground">{introTitle}</h3>
        <p className="mt-1 text-sm text-muted-foreground">{introBody}</p>
        {showQuarantine ? (
          <p className="mt-2 text-sm text-muted-foreground">
            {isAlloy
              ? `Si un miembro es Mirror, las versiones publicadas en el upstream hace menos de ${quarantineDays} días no aparecen en el índice y npm install recibe 403 hasta que cumplan esa edad.`
              : `Las versiones publicadas en el upstream hace menos de ${quarantineDays} días no aparecen en el índice y npm install recibe 403 hasta que cumplan esa edad.`}{" "}
            Reduce la ventana de typosquatting. Los Forges no se retienen. Se configura con{" "}
            <code className="font-mono">MIRROR_QUARANTINE_DAYS</code> (0 la desactiva).
          </p>
        ) : null}
      </div>

      <div className="space-y-2">
        <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          1. Apunta npm a este registro
        </p>
        <CopyableCodeBlock
          code={`registry=${registryUrl}\n//${authHost}:_authToken=fb_…`}
        />
        <p className="text-xs text-muted-foreground">
          Ponlo en <code className="font-mono">.npmrc</code> del paquete o en{" "}
          <code className="font-mono">~/.npmrc</code>. El token se crea en Seguridad. La barra
          final de la URL es obligatoria.
        </p>
      </div>

      {readOnly ? (
        <div className="space-y-2">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            2. Instala un paquete
          </p>
          <CopyableCodeBlock code="npm install demo-pkg" />
        </div>
      ) : (
        <>
          <div className="space-y-2">
            <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
              2. Publica tu paquete
            </p>
            <CopyableCodeBlock code="npm publish" />
          </div>
          <div className="space-y-2">
            <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
              3. Instálalo en otro proyecto
            </p>
            <CopyableCodeBlock code="npm install demo-pkg" />
          </div>
        </>
      )}
    </div>
  );

  if (!framed) {
    return body;
  }

  return <div className="rounded-lg border border-border p-5">{body}</div>;
}
