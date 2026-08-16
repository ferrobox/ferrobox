CREATE TABLE groups (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE group_members (
    group_id UUID NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    PRIMARY KEY (group_id, user_id)
);

CREATE INDEX idx_group_members_user_id ON group_members (user_id);

CREATE TABLE repository_group_access (
    repository_id UUID NOT NULL REFERENCES repositories (id) ON DELETE CASCADE,
    group_id UUID NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('reader', 'developer')),
    PRIMARY KEY (repository_id, group_id)
);

CREATE INDEX idx_repository_group_access_group_id ON repository_group_access (group_id);
