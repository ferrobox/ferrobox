ALTER TABLE users
    ADD COLUMN role TEXT NOT NULL DEFAULT 'admin'
    CONSTRAINT users_role_check CHECK (role IN ('admin', 'developer', 'reader'));

ALTER TABLE users
    ALTER COLUMN role SET DEFAULT 'developer';
