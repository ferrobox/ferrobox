import { type FormEvent, useState } from "react";
import { AlertCircle, Copy, KeyRound, RefreshCw, Trash2 } from "lucide-react";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useApiTokens, useCreateApiToken, useRevokeApiToken } from "@/api/queries";
import { PageHeader } from "@/components/layout/PageHeader";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

export function SecurityPage() {
  const { data, isPending, isError, error, refetch, isFetching } = useApiTokens();
  const createToken = useCreateApiToken();
  const revokeToken = useRevokeApiToken();
  const [name, setName] = useState("");
  const [createdSecret, setCreatedSecret] = useState<string | null>(null);

  async function onCreate(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    try {
      const result = await createToken.mutateAsync({ name: name.trim() });
      setCreatedSecret(result.token);
      setName("");
      toast.success("Token creado");
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo crear el token");
    }
  }

  async function onRevoke(tokenId: string, tokenName: string) {
    try {
      await revokeToken.mutateAsync(tokenId);
      toast.success(`Token «${tokenName}» revocado`);
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo revocar el token");
    }
  }

  return (
    <div>
      <PageHeader
        title="Seguridad"
        description="Emite y revoca tokens de API para la UI, cargo publish y automatizaciones."
      />

      <form
        onSubmit={(event) => void onCreate(event)}
        className="mb-8 flex flex-col gap-3 sm:flex-row sm:items-end"
      >
        <div className="w-full space-y-2 sm:max-w-sm">
          <Label htmlFor="token-name">Nombre del token</Label>
          <Input
            id="token-name"
            placeholder="cargo-publish"
            value={name}
            onChange={(event) => setName(event.target.value)}
            required
          />
        </div>
        <Button type="submit" disabled={createToken.isPending || name.trim().length === 0}>
          <KeyRound />
          {createToken.isPending ? "Creando…" : "Crear token"}
        </Button>
      </form>

      {isPending ? (
        <div className="space-y-2">
          {Array.from({ length: 3 }, (_, index) => (
            <Skeleton key={index} className="h-12 w-full rounded-lg" />
          ))}
        </div>
      ) : null}

      {isError ? (
        <Alert variant="destructive">
          <AlertCircle />
          <AlertTitle>No se pudieron cargar los tokens</AlertTitle>
          <AlertDescription className="flex items-center justify-between gap-4">
            <span>{error.message}</span>
            <Button size="sm" variant="outline" onClick={() => void refetch()}>
              <RefreshCw className={isFetching ? "animate-spin" : ""} />
              Reintentar
            </Button>
          </AlertDescription>
        </Alert>
      ) : null}

      {data && data.length === 0 ? (
        <div className="rounded-lg border border-dashed border-border py-16 text-center">
          <p className="font-medium text-foreground">Todavía no hay tokens</p>
          <p className="mt-1 text-sm text-muted-foreground">
            Crea uno para autenticar `cargo publish` o integraciones.
          </p>
        </div>
      ) : null}

      {data && data.length > 0 ? (
        <div className="overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Nombre</TableHead>
                <TableHead>Prefijo</TableHead>
                <TableHead>Creado</TableHead>
                <TableHead className="text-right">Acciones</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.map((token) => (
                <TableRow key={token.id}>
                  <TableCell className="font-medium">{token.name}</TableCell>
                  <TableCell className="font-mono text-xs text-muted-foreground">
                    {token.prefix}…
                  </TableCell>
                  <TableCell className="text-sm text-muted-foreground">
                    {formatCreatedAt(token.created_at)}
                  </TableCell>
                  <TableCell className="text-right">
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={revokeToken.isPending}
                      onClick={() => void onRevoke(token.id, token.name)}
                    >
                      <Trash2 />
                      Revocar
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      ) : null}

      <Dialog open={createdSecret !== null} onOpenChange={(open) => !open && setCreatedSecret(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Copia el token ahora</DialogTitle>
            <DialogDescription>
              Este secreto solo se muestra una vez. Úsalo como Bearer token o en
              `~/.cargo/credentials.toml`.
            </DialogDescription>
          </DialogHeader>
          <div className="flex items-center gap-2">
            <Input readOnly value={createdSecret ?? ""} className="font-mono text-xs" />
            <Button
              type="button"
              variant="outline"
              size="icon"
              aria-label="Copiar token"
              onClick={() => {
                if (createdSecret) {
                  void navigator.clipboard.writeText(createdSecret);
                  toast.success("Token copiado");
                }
              }}
            >
              <Copy />
            </Button>
          </div>
          <DialogFooter>
            <Button type="button" onClick={() => setCreatedSecret(null)}>
              He guardado el token
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function formatCreatedAt(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleString();
}
