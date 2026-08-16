ALTER TABLE users
    ADD COLUMN email TEXT;

CREATE UNIQUE INDEX users_email_key ON users (email);
