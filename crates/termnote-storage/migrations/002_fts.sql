-- Full-text search over commands, output, notes and bookmark names
-- (PRD section 43). Kept as a contentless-adjacent external-content table so
-- the searchable text is duplicated once into the FTS index rather than
-- doubling storage of large output blobs.

CREATE VIRTUAL TABLE IF NOT EXISTS events_fts USING fts5(
    content,
    session_id UNINDEXED,
    event_id UNINDEXED,
    event_type UNINDEXED,
    tokenize = 'unicode61'
);
