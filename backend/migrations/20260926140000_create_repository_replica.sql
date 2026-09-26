CREATE TABLE repository_replica (
    repository_id UUID PRIMARY KEY REFERENCES repositories (id) ON DELETE CASCADE,
    remote_url TEXT NOT NULL,
    destination_id UUID NOT NULL,
    token TEXT,
    last_run_at TIMESTAMPTZ,
    last_packages_imported INTEGER,
    last_artifacts_imported INTEGER,
    last_skipped INTEGER,
    last_error TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
