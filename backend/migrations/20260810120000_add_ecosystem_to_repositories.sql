ALTER TABLE repositories
    ADD COLUMN ecosystem TEXT NOT NULL DEFAULT 'generic'
        CHECK (ecosystem IN ('generic', 'cargo', 'npm', 'pypi', 'oci', 'helm'));

ALTER TABLE repositories ALTER COLUMN ecosystem DROP DEFAULT;

CREATE INDEX idx_repositories_ecosystem ON repositories (ecosystem);
