CREATE TABLE repositories (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (kind IN ('forge', 'mirror', 'alloy')),
    kind_data JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_repositories_kind ON repositories (kind);
