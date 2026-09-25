import { type FormEvent, useState } from "react";
import { AlertCircle, Search } from "lucide-react";
import { useTranslation } from "react-i18next";
import { NavLink, useSearchParams } from "react-router-dom";

import { usePackageSearch } from "@/api/queries";
import { PageHeader } from "@/components/layout/PageHeader";
import { EcosystemBadge } from "@/components/repository/EcosystemBadge";
import { RepositoryKindBadge } from "@/components/repository/RepositoryKindBadge";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

export function SearchPage() {
  const { t } = useTranslation();
  const [params, setParams] = useSearchParams();
  const urlQuery = params.get("q") ?? "";
  const { data, isPending, isError, error, isFetching } = usePackageSearch(urlQuery);

  const hits = data?.hits ?? [];
  const searching = urlQuery.trim().length > 0;

  function onSearch(next: string) {
    const trimmed = next.trim();
    if (trimmed.length === 0) {
      setParams({}, { replace: true });
      return;
    }
    setParams({ q: trimmed }, { replace: true });
  }

  return (
    <div>
      <PageHeader title={t("search.title")} description={t("search.description")} />

      <SearchForm key={urlQuery} initial={urlQuery} onSearch={onSearch} />

      {!searching ? (
        <div className="rounded-lg border border-dashed border-border py-16 text-center">
          <Search className="mx-auto size-8 text-muted-foreground" />
          <p className="mt-3 font-medium text-foreground">{t("search.emptyTitle")}</p>
          <p className="mt-1 text-sm text-muted-foreground">{t("search.emptyBody")}</p>
        </div>
      ) : null}

      {searching && (isPending || isFetching) && !data ? (
        <div className="space-y-2">
          {Array.from({ length: 4 }, (_, index) => (
            <Skeleton key={index} className="h-12 w-full rounded-lg" />
          ))}
        </div>
      ) : null}

      {isError ? (
        <Alert variant="destructive">
          <AlertCircle />
          <AlertTitle>{t("search.failed")}</AlertTitle>
          <AlertDescription>{error.message}</AlertDescription>
        </Alert>
      ) : null}

      {searching && data && hits.length === 0 && !isFetching ? (
        <div className="rounded-lg border border-dashed border-border py-16 text-center">
          <p className="font-medium text-foreground">{t("search.noHits", { query: urlQuery })}</p>
          <p className="mt-1 text-sm text-muted-foreground">{t("search.noHitsHint")}</p>
        </div>
      ) : null}

      {hits.length > 0 ? (
        <div className="overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("common.package")}</TableHead>
                <TableHead>{t("common.repository")}</TableHead>
                <TableHead>{t("common.ecosystem")}</TableHead>
                <TableHead>{t("common.type")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {hits.map((hit) => (
                <TableRow key={`${hit.repository_id}:${hit.name}`}>
                  <TableCell>
                    <NavLink
                      to={`/repositories/${hit.repository_id}`}
                      className={`font-mono text-sm underline-offset-4 hover:underline ${hit.yanked ? "text-muted-foreground line-through" : "text-foreground"}`}
                    >
                      {hit.name}@{hit.version}
                    </NavLink>
                    {hit.yanked ? (
                      <Badge variant="outline" className="ml-2">
                        yanked
                      </Badge>
                    ) : null}
                  </TableCell>
                  <TableCell>
                    <NavLink
                      to={`/repositories/${hit.repository_id}`}
                      className="text-sm underline-offset-4 hover:underline"
                    >
                      {hit.repository_name}
                    </NavLink>
                  </TableCell>
                  <TableCell>
                    <EcosystemBadge ecosystem={hit.ecosystem} />
                  </TableCell>
                  <TableCell>
                    <RepositoryKindBadge kind={hit.kind} />
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      ) : null}
    </div>
  );
}

function SearchForm({
  initial,
  onSearch,
}: {
  initial: string;
  onSearch: (query: string) => void;
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState(initial);

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    onSearch(draft);
  }

  return (
    <form onSubmit={onSubmit} className="mb-6 flex max-w-xl gap-2">
      <Input
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        placeholder={t("search.placeholder")}
        aria-label={t("search.placeholder")}
      />
      <Button type="submit" variant="outline">
        <Search />
        {t("search.submit")}
      </Button>
    </form>
  );
}
