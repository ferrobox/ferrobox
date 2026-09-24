CREATE TABLE repository_admission_events (
    id UUID PRIMARY KEY,
    repository_id UUID NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    reference TEXT NOT NULL,
    effect TEXT NOT NULL,
    reason TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT repository_admission_events_effect_chk CHECK (effect IN ('deny', 'warn'))
);

CREATE INDEX repository_admission_events_repo_created_idx
    ON repository_admission_events (repository_id, created_at DESC);
