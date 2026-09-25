ALTER TABLE users
    ADD COLUMN oidc_issuer TEXT,
    ADD COLUMN oidc_subject TEXT;

CREATE UNIQUE INDEX users_oidc_identity_key
    ON users (oidc_issuer, oidc_subject)
    WHERE oidc_subject IS NOT NULL;

CREATE TABLE sso_group_memberships (
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    group_id UUID NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, group_id)
);

CREATE INDEX idx_sso_group_memberships_group_id ON sso_group_memberships (group_id);
