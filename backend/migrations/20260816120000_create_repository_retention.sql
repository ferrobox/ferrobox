CREATE TABLE repository_retention (
    repository_id UUID PRIMARY KEY REFERENCES repositories(id) ON DELETE CASCADE,
    keep_last INTEGER CHECK (keep_last IS NULL OR keep_last >= 1),
    keep_days INTEGER CHECK (keep_days IS NULL OR keep_days >= 1),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
