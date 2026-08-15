ALTER TABLE repositories DROP CONSTRAINT repositories_ecosystem_check;

ALTER TABLE repositories
    ADD CONSTRAINT repositories_ecosystem_check
        CHECK (ecosystem IN ('generic', 'cargo', 'npm', 'pypi', 'oci', 'helm', 'conan'));
