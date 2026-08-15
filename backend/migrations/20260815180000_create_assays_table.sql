CREATE TABLE assays (
    id UUID PRIMARY KEY,
    repository_id UUID NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    ecosystem TEXT NOT NULL,
    package_name TEXT NOT NULL,
    package_version TEXT NOT NULL,
    status TEXT NOT NULL,
    error_message TEXT,
    scanned_at TIMESTAMPTZ,
    critical_count INTEGER NOT NULL DEFAULT 0,
    high_count INTEGER NOT NULL DEFAULT 0,
    medium_count INTEGER NOT NULL DEFAULT 0,
    low_count INTEGER NOT NULL DEFAULT 0,
    unknown_count INTEGER NOT NULL DEFAULT 0,
    component_count INTEGER NOT NULL DEFAULT 0,
    components JSONB NOT NULL DEFAULT '[]'::jsonb,
    findings JSONB NOT NULL DEFAULT '[]'::jsonb,
    UNIQUE (repository_id, ecosystem, package_name, package_version)
);

CREATE INDEX idx_assays_repository ON assays (repository_id);
