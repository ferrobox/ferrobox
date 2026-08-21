CREATE TABLE webhooks (
    id UUID PRIMARY KEY,
    repository_id UUID NOT NULL REFERENCES repositories (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    secret TEXT,
    events TEXT[] NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_webhooks_repository_id ON webhooks (repository_id);

CREATE TABLE webhook_deliveries (
    id UUID PRIMARY KEY,
    webhook_id UUID NOT NULL REFERENCES webhooks (id) ON DELETE CASCADE,
    event TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('success', 'failed')),
    http_status INTEGER,
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_webhook_deliveries_webhook_id_created_at
    ON webhook_deliveries (webhook_id, created_at DESC);
