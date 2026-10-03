CREATE TABLE osv_sync (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE,
    reference TEXT NOT NULL,
    public_key_pem TEXT NOT NULL,
    last_outcome TEXT,
    last_detail TEXT,
    last_attempt_at TIMESTAMPTZ,
    retry_after TIMESTAMPTZ,
    CONSTRAINT osv_sync_singleton_chk CHECK (singleton)
);
