import { useQuery } from "@tanstack/react-query";

import { cn } from "@/lib/utils";

async function fetchHealth(): Promise<boolean> {
  const response = await fetch("/api/health");
  return response.ok;
}

export function HealthIndicator() {
  const { data: healthy, isFetching } = useQuery({
    queryKey: ["health"],
    queryFn: fetchHealth,
    refetchInterval: 30_000,
    retry: false,
  });

  const status = isFetching && healthy === undefined ? "checking" : healthy ? "ok" : "down";

  const label =
    status === "checking"
      ? "Comprobando conexión…"
      : status === "ok"
        ? "Backend conectado"
        : "Backend no disponible";

  return (
    <div className="flex items-center gap-2 text-sm text-muted-foreground">
      <span
        className={cn(
          "size-2 rounded-full",
          status === "ok" && "bg-success",
          status === "down" && "bg-destructive",
          status === "checking" && "animate-pulse bg-muted-foreground/40",
        )}
      />
      {label}
    </div>
  );
}
