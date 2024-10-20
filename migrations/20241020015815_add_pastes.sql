-- Add migration script here
CREATE TABLE pastes (
    id INTEGER PRIMARY KEY,
    expires INTEGER,
    text BLOB NOT NULL
);
