-- Add migration script here
CREATE TABLE pastes (
    id INTEGER PRIMARY KEY,
    expires INTEGER,
    text BLOB NOT NULL
);

UPDATE images SET created = created + 1209600;

ALTER TABLE images RENAME created TO expires;
