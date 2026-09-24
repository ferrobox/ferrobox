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

export function OciRegistryPanel({
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

  const baseUrl = data.public_base_url.replace(/\/$/, "");
  const registryHost = baseUrl.replace(/^https?:\/\//, "");
  const imageRef = `${registryHost}/${repositoryId}/demo:latest`;
  const isMirror = kind === "mirror";
  const isAlloy = kind === "alloy";
  const readOnly = isMirror || isAlloy;

  const introTitle = isAlloy ? "Alloy OCI" : isMirror ? "Mirror OCI" : "Registro OCI";
  const introBody = isAlloy
    ? "Este Alloy agrega Forges y/o Mirrors OCI en una sola URL. docker pull resuelve contra los miembros, en orden. No acepta docker push: publica en un Forge miembro."
    : isMirror
      ? "Este Mirror cachea imágenes del upstream (por ejemplo Docker Hub) la primera vez que docker pull las resuelve. No acepta docker push ni yank."
      : "Este repositorio implementa el Distribution Spec v2: docker push y docker pull funcionan de forma nativa.";

  const body = (
    <div className="space-y-5">
      <div>
        <h3 className="text-sm font-semibold text-foreground">{introTitle}</h3>
        <p className="mt-1 text-sm text-muted-foreground">{introBody}</p>
      </div>

      {readOnly ? null : (
        <div className="space-y-2">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            1. Login (token de Seguridad)
          </p>
          <CopyableCodeBlock
            code={`echo 'fb_…' | docker login ${registryHost} -u __token__ --password-stdin`}
          />
          <p className="text-xs text-muted-foreground">
            En HTTP local Docker exige{" "}
            <code className="font-mono">insecure-registries: [&quot;{registryHost}&quot;]</code>{" "}
            en <code className="font-mono">/etc/docker/daemon.json</code> y reiniciar el daemon.
            Copia el UUID completo (8-4-4-4-12). Si un <code className="font-mono">login</code>{" "}
            antiguo no basta para el push, haz{" "}
            <code className="font-mono">docker logout {registryHost}</code> y vuelve a entrar.
          </p>
        </div>
      )}
      <div className="space-y-2">
        <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          {readOnly ? "1. Instala (pull)" : "2. Publica y tira"}
        </p>
        <CopyableCodeBlock
          code={
            readOnly
              ? `docker pull ${imageRef}`
              : `docker tag alpine:latest ${imageRef}\ndocker push ${imageRef}\ndocker pull ${imageRef}`
          }
        />
        <p className="text-xs text-muted-foreground">
          El nombre de imagen es <code className="font-mono">&lt;UUID&gt;/demo</code> (también vale
          con barra, p. ej. <code className="font-mono">bitnami/nginx</code>). El registro es la
          raíz del host (<code className="font-mono">/v2/</code>), no un prefijo extra.
          {isMirror ? (
            <>
              {" "}
              Con upstream Docker Hub, <code className="font-mono">alpine:latest</code> se
              resuelve como <code className="font-mono">library/alpine</code>.
            </>
          ) : null}
        </p>
      </div>
      {readOnly ? (
        <div className="space-y-2">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            2. Verificar firma Cosign
          </p>
          <CopyableCodeBlock
            code={`cosign verify --key cosign.pub ${imageRef}`}
          />
          <p className="text-xs text-muted-foreground">
            FerroBox expone el Referrers API y las etiquetas{" "}
            <code className="font-mono">sha256-&lt;digest&gt;.sig</code>. Cosign las resuelve contra
            el mismo registro. En HTTP local añade{" "}
            <code className="font-mono">--allow-insecure-registry</code>.
          </p>
        </div>
      ) : (
        <div className="space-y-2">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            3. Firmar y verificar (Cosign)
          </p>
          <CopyableCodeBlock
            code={`cosign generate-key-pair
cosign sign --key cosign.key ${imageRef}
cosign verify --key cosign.pub ${imageRef}`}
          />
          <p className="text-xs text-muted-foreground">
            La firma queda como accesorio OCI de la imagen (etiqueta{" "}
            <code className="font-mono">.sig</code> o Referrers API). La UI muestra el badge
            Firmada. En HTTP local añade{" "}
            <code className="font-mono">--allow-insecure-registry</code>. Este paso no exige la
            firma al hacer <code className="font-mono">docker pull</code>.
          </p>
        </div>
      )}
    </div>
  );

  if (!framed) {
    return body;
  }

  return <div className="rounded-lg border border-border p-5">{body}</div>;
}
