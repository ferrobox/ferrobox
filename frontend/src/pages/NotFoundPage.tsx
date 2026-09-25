import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";

import { Button } from "@/components/ui/button";

export default function NotFoundPage({ message }: { message?: string }) {
  const { t } = useTranslation();

  return (
    <div className="flex flex-col items-center justify-center gap-4 py-24 text-center">
      <p className="text-5xl font-semibold tracking-tight text-foreground">404</p>
      <p className="text-sm text-muted-foreground">{message ?? t("notFound.default")}</p>
      <Button asChild variant="outline">
        <Link to="/repositories">{t("notFound.back")}</Link>
      </Button>
    </div>
  );
}
