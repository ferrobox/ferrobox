ALTER TABLE repository_admission
    ADD COLUMN public_keys_pem TEXT NOT NULL DEFAULT '';

ALTER TABLE repository_admission
    DROP CONSTRAINT repository_admission_predicate_chk;

ALTER TABLE repository_admission
    ADD CONSTRAINT repository_admission_predicate_chk
    CHECK (predicate IN ('not_signed', 'not_verified'));
