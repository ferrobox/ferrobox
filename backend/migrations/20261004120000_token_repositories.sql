ALTER TABLE api_tokens
    ADD COLUMN repository_ids TEXT NOT NULL DEFAULT '';
