CREATE TABLE repository_quota (
    repository_id UUID PRIMARY KEY REFERENCES repositories(id) ON DELETE CASCADE,
    limit_bytes BIGINT CHECK (limit_bytes IS NULL OR limit_bytes >= 1),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
