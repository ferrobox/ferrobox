import { Check, Copy } from "lucide-react";
import { useState } from "react";

import { useCargoRegistryConfig } from "@/api/queries";
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

export function CargoRegistryPanel({ repositoryId }: { repositoryId: string }) {
  const { data, isPending, isError } = useCargoRegistryConfig(repositoryId, true);

  if (isPending) {
    return <Skeleton className="h-40 w-full rounded-lg" />;
  }

  if (isError || !data) {
    return (
      <p className="text-sm text-muted-foreground">
        No se pudo obtener la configuración del registro de Cargo.
      </p>
    );
  }

  const indexUrl = `sparse+${data.api}/`;

  return (
    <div className="space-y-5 rounded-lg border border-border p-5">
      <div>
        <h3 className="text-sm font-semibold text-foreground">Índice disperso</h3>
        <p className="mt-1 text-sm text-muted-foreground">
          Este repositorio implementa el protocolo de índice disperso de Cargo, así que
          funciona con <code className="font-mono text-xs">cargo publish</code>,{" "}
          <code className="font-mono text-xs">cargo add</code> y{" "}
          <code className="font-mono text-xs">cargo build</code> de forma nativa.
        </p>
      </div>

      <div className="space-y-2">
        <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          1. Registra el índice en tu configuración de Cargo
        </p>
        <CopyableCodeBlock
          code={`[registries.ferrobox]\nindex = "${indexUrl}"`}
        />
        <p className="text-xs text-muted-foreground">
          Añádelo a <code className="font-mono">~/.cargo/config.toml</code> o a{" "}
          <code className="font-mono">.cargo/config.toml</code> dentro de tu proyecto.
        </p>
      </div>

      <div className="space-y-2">
        <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          2. Guarda un token de API
        </p>
        <CopyableCodeBlock
          code={`[registry-tokens]\nferrobox = "fb_…"  # créalo en Seguridad`}
        />
        <p className="text-xs text-muted-foreground">
          Emite el token en <span className="font-medium">Seguridad</span> y
          añádelo a <code className="font-mono">~/.cargo/credentials.toml</code>.
        </p>
      </div>

      <div className="space-y-2">
        <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          3. Publica tu crate
        </p>
        <CopyableCodeBlock code="cargo publish --registry ferrobox" />
      </div>

      <div className="space-y-2">
        <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          4. Añádelo como dependencia
        </p>
        <CopyableCodeBlock code="cargo add mi-crate --registry ferrobox" />
      </div>
    </div>
  );
}
