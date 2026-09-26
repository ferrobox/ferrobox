ALTER TABLE repository_replica
    ADD COLUMN direction TEXT NOT NULL DEFAULT 'push';
