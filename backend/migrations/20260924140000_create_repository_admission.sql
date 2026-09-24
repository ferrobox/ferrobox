CREATE TABLE repository_admission (
    repository_id UUID PRIMARY KEY REFERENCES repositories(id) ON DELETE CASCADE,
    enabled BOOLEAN NOT NULL DEFAULT FALSE,
    moment TEXT NOT NULL DEFAULT 'pull',
    predicate TEXT NOT NULL DEFAULT 'not_signed',
    effect TEXT NOT NULL DEFAULT 'deny',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT repository_admission_moment_chk CHECK (moment IN ('pull')),
    CONSTRAINT repository_admission_predicate_chk CHECK (predicate IN ('not_signed')),
    CONSTRAINT repository_admission_effect_chk CHECK (effect IN ('deny', 'warn'))
);
