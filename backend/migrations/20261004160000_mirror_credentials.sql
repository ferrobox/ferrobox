CREATE TABLE mirror_credentials (
    repository_id UUID PRIMARY KEY REFERENCES repositories (id) ON DELETE CASCADE,
    username TEXT NOT NULL DEFAULT '',
    secret TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
