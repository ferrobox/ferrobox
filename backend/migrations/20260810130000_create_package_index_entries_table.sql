CREATE TABLE package_index_entries (
    repository_id UUID NOT NULL REFERENCES repositories(id),
    ecosystem TEXT NOT NULL,
    package_name TEXT NOT NULL,
    package_version TEXT NOT NULL,
    artifact_id UUID NOT NULL REFERENCES artifacts(id),
    entry JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (repository_id, ecosystem, package_name, package_version)
);

CREATE INDEX idx_package_index_entries_lookup
    ON package_index_entries (repository_id, ecosystem, lower(package_name));
