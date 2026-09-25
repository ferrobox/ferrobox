import { Languages } from "lucide-react";
import { useTranslation } from "react-i18next";

import { SUPPORTED_LOCALES, type SupportedLocale } from "@/i18n";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

export function LanguageSwitcher({ compact = true }: { compact?: boolean }) {
  const { t, i18n } = useTranslation();
  const current: SupportedLocale = i18n.resolvedLanguage?.startsWith("es") ? "es" : "en";

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size={compact ? "icon" : "sm"}
          aria-label={t("topbar.language")}
          title={t("topbar.language")}
        >
          <Languages className="size-4" />
          {compact ? null : t("topbar.language")}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuRadioGroup
          value={current}
          onValueChange={(value) => {
            void i18n.changeLanguage(value);
          }}
        >
          {SUPPORTED_LOCALES.map((locale) => (
            <DropdownMenuRadioItem key={locale} value={locale}>
              {locale === "en" ? t("topbar.english") : t("topbar.spanish")}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
