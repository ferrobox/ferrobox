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

export function CargoRegistryPanel({
  repositoryId,
  kind = "forge",
  framed = true,
}: {
  repositoryId: string;
  kind?: "forge" | "mirror" | "alloy";
  framed?: boolean;
}) {
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
  const isMirror = kind === "mirror";
  const isAlloy = kind === "alloy";
  const readOnly = isMirror || isAlloy;

  const introTitle = isAlloy
    ? "Alloy de índice disperso"
    : isMirror
      ? "Mirror de índice disperso"
      : "Índice disperso";
  const introBody = isAlloy
    ? "Este Alloy agrega Forges y/o Mirrors Cargo en una sola URL. cargo add y cargo build resuelven contra los miembros, en orden; el primero gana si hay la misma versión. No acepta cargo publish ni cargo yank: publica en un Forge miembro."
    : isMirror
      ? "Este Mirror cachea paquetes del upstream la primera vez que se resuelven o descargan. No acepta cargo publish ni cargo yank."
      : "Este repositorio implementa el protocolo de índice disperso de Cargo: cargo publish, cargo yank, cargo search, cargo add y cargo build funcionan de forma nativa.";

  const body = (
    <div className="space-y-5">
      <div>
        <h3 className="text-sm font-semibold text-foreground">
          {introTitle}
        </h3>
        <p className="mt-1 text-sm text-muted-foreground">{introBody}</p>
      </div>

      <div className="space-y-2">
        <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          1. Registra el índice en tu configuración de Cargo
        </p>
        <CopyableCodeBlock
          code={`[registries.ferrobox]\nindex = "${indexUrl}"`}
        />
        <p className="text-xs text-muted-foreground">
          Puedes ponerlo en <code className="font-mono">~/.cargo/config.toml</code> o en{" "}
          <code className="font-mono">.cargo/config.toml</code> del crate (por ejemplo{" "}
          <code className="font-mono">demo-ferrobox/.cargo/config.toml</code>).
        </p>
      </div>

      {!readOnly ? (
        <div className="space-y-2">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            2. Guarda un token de API (solo para publicar o hacer yank)
          </p>
          <CopyableCodeBlock
            code={`[registries.ferrobox]\ntoken = "fb_…"  # créalo en Seguridad`}
          />
          <p className="text-xs text-muted-foreground">
            El índice y las descargas son públicos: <code className="font-mono">cargo add</code> y{" "}
            <code className="font-mono">cargo build</code> no necesitan token. Emite uno en{" "}
            <span className="font-medium">Seguridad</span> y guárdalo en{" "}
            <code className="font-mono">~/.cargo/credentials.toml</code> (Cargo no lee{" "}
            <code className="font-mono">.cargo/credentials.toml</code> del proyecto). Alternativa:{" "}
            <code className="font-mono">cargo login --registry ferrobox</code> o{" "}
            <code className="font-mono">cargo publish --registry ferrobox --token fb_…</code>. No
            subas el token al repositorio.
          </p>
        </div>
      ) : null}

      {readOnly ? (
        <div className="space-y-2">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            2. Usa crates desde este registro
          </p>
          <CopyableCodeBlock code="cargo add serde --registry ferrobox" />
          {isAlloy ? (
            <p className="text-xs text-muted-foreground">
              Para publicar, apunta <code className="font-mono">cargo publish --registry</code> a
              un Forge miembro, no a este Alloy.
            </p>
          ) : null}
        </div>
      ) : (
        <>
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

          <div className="space-y-2">
            <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
              5. Yank de una versión
            </p>
            <CopyableCodeBlock code="cargo yank --vers 0.1.0 --registry ferrobox" />
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
