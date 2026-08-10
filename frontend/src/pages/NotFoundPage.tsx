import { Link } from "react-router-dom";

import { Button } from "@/components/ui/button";

export default function NotFoundPage({
  message = "La página que buscas no existe.",
}: {
  message?: string;
}) {
  return (
    <div className="flex flex-col items-center justify-center gap-4 py-24 text-center">
      <p className="text-5xl font-semibold tracking-tight text-foreground">404</p>
      <p className="text-sm text-muted-foreground">{message}</p>
      <Button asChild variant="outline">
        <Link to="/repositories">Volver a repositorios</Link>
      </Button>
    </div>
  );
}
