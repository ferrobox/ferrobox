CREATE TABLE osv_feed (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE,
    dataset TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    advisory_count BIGINT NOT NULL,
    ecosystems TEXT NOT NULL,
    storage_key TEXT NOT NULL,
    imported_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT osv_feed_singleton_chk CHECK (singleton)
);
